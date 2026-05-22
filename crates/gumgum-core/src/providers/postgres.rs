use tokio::process::Command as TokioCommand;

use super::docker::{
    create_provider_container, ensure_network, inspect, run_provider_command, start_existing,
};
use crate::{Capability, sanitize_name};

use super::types::{ObjectProviderPlan, ProviderCredentials, ProviderSpec};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Db,
        provider: "postgres.main".to_owned(),
        container: "gumgum-provider-postgres-main".to_owned(),
        image: "postgres:16-alpine".to_owned(),
        port: 5432,
        protocol: "postgres".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure postgres.main provider is running".to_owned(),
        format!("ensure database {safe_name} exists"),
        format!("publish DNS {dns} to postgres.main"),
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("psql postgres://{name}:<password>@{dns}:5432/{name}"),
        format!("pgAdmin host={dns} port=5432 database={name} username={name}"),
    ]
}

pub(crate) async fn ensure_object(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let mut actions = ensure(&plan.provider, credentials.clone()).await?;
    let database = sanitize_name(&plan.name);
    let owner = sanitize_name(&plan.name);
    if let Some(password) = &plan.object_password {
        ensure_role(&plan.provider, &credentials, &owner, password).await?;
        actions.push(format!("ensured database role {owner}"));
    }
    if database_exists(&plan.provider, &credentials, &database).await? {
        actions.push(format!("database {database} already exists"));
    } else {
        create_database(
            &plan.provider,
            &credentials,
            &database,
            plan.object_password.as_deref().map(|_| owner.as_str()),
        )
        .await?;
        actions.push(format!("created database {database}"));
    }
    if plan.object_password.is_some() {
        grant_database(&plan.provider, &credentials, &database, &owner).await?;
        actions.push(format!("granted database {database} to role {owner}"));
    }
    actions.push(format!("published DNS {} to postgres.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete_object(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let mut actions = ensure(&plan.provider, credentials.clone()).await?;
    let database = sanitize_name(&plan.name);
    if database_exists(&plan.provider, &credentials, &database).await? {
        drop_database(&plan.provider, &credentials, &database).await?;
        actions.push(format!("dropped database {database}"));
    } else {
        actions.push(format!("database {database} was already absent"));
    }
    actions.push(format!("removed DNS {} from postgres.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn ensure(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start postgres provider").await;
    }
    create_provider_container(
        provider,
        vec![
            (credentials.username_env, credentials.username),
            (credentials.password_env, credentials.password),
        ],
        Vec::new(),
    )
    .await
}

async fn database_exists(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
) -> crate::Result<bool> {
    let output = TokioCommand::new("docker")
        .arg("exec")
        .arg("-e")
        .arg(format!("PGPASSWORD={}", credentials.password))
        .arg(&provider.container)
        .arg("psql")
        .arg("-U")
        .arg(&credentials.username)
        .arg("-tAc")
        .arg(format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            shell_single_quote(database)
        ))
        .output()
        .await
        .map_err(|source| {
            crate::GumgumError::structured(
                crate::Subsystem::Setup,
                crate::ErrorCode::Io,
                "could not inspect postgres database",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
    } else {
        Err(crate::GumgumError::structured(
            crate::Subsystem::Setup,
            crate::ErrorCode::Io,
            "could not inspect postgres database",
        )
        .likely_cause(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        .build())
    }
}

async fn create_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
    owner: Option<&str>,
) -> crate::Result<()> {
    let mut command = TokioCommand::new("docker");
    command
        .arg("exec")
        .arg("-e")
        .arg(format!("PGPASSWORD={}", credentials.password))
        .arg(&provider.container)
        .arg("createdb")
        .arg("-U")
        .arg(&credentials.username);
    if let Some(owner) = owner {
        command.arg("-O").arg(owner);
    }
    command.arg(database);
    run_provider_command(&mut command, "could not create postgres database").await
}

async fn ensure_role(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    role: &str,
    password: &str,
) -> crate::Result<()> {
    run_psql(
        provider,
        credentials,
        &format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{}') THEN CREATE ROLE \"{}\" LOGIN PASSWORD '{}'; ELSE ALTER ROLE \"{}\" WITH LOGIN PASSWORD '{}'; END IF; END $$;",
            shell_single_quote(role),
            sql_identifier(role),
            shell_single_quote(password),
            sql_identifier(role),
            shell_single_quote(password)
        ),
        "could not ensure postgres object role",
    )
    .await
}

async fn grant_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
    role: &str,
) -> crate::Result<()> {
    run_psql(
        provider,
        credentials,
        &format!(
            "GRANT ALL PRIVILEGES ON DATABASE \"{}\" TO \"{}\";",
            sql_identifier(database),
            sql_identifier(role)
        ),
        "could not grant postgres database privileges",
    )
    .await
}

async fn run_psql(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    sql: &str,
    error: &str,
) -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("exec")
            .arg("-e")
            .arg(format!("PGPASSWORD={}", credentials.password))
            .arg(&provider.container)
            .arg("psql")
            .arg("-U")
            .arg(&credentials.username)
            .arg("-d")
            .arg("postgres")
            .arg("-c")
            .arg(sql),
        error,
    )
    .await
}

async fn drop_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
) -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("exec")
            .arg("-e")
            .arg(format!("PGPASSWORD={}", credentials.password))
            .arg(&provider.container)
            .arg("dropdb")
            .arg("--if-exists")
            .arg("-U")
            .arg(&credentials.username)
            .arg(database),
        "could not drop postgres database",
    )
    .await
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn sql_identifier(value: &str) -> String {
    value.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_object_actions_include_database_and_dns() {
        assert_eq!(
            actions("visit-counter", "visits.db.leostera.dev"),
            vec![
                "ensure postgres.main provider is running",
                "ensure database visit-counter exists",
                "publish DNS visits.db.leostera.dev to postgres.main",
            ]
        );
    }

    #[test]
    fn postgres_delete_for_absent_database_is_actionable() {
        let plan = crate::providers::object_provider_plan(
            Capability::Db,
            "visits",
            "visits.db.leostera.dev",
        );

        assert_eq!(plan.provider.provider, "postgres.main");
    }

    #[test]
    fn postgres_sql_literal_escapes_single_quotes() {
        assert_eq!(shell_single_quote("team's-db"), "team''s-db");
    }

    #[test]
    fn postgres_sql_identifier_escapes_double_quotes() {
        assert_eq!(sql_identifier("team\"db"), "team\"\"db");
    }
}
