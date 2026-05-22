use super::docker::{create_provider_container, ensure_network, inspect, start_existing};
use crate::{Capability, DockerEngine, sanitize_name};

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
    let output = DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "psql".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                "-tAc".to_owned(),
                format!(
                    "SELECT 1 FROM pg_database WHERE datname = '{}'",
                    shell_single_quote(database)
                ),
            ],
        )
        .await?;
    Ok(output.trim() == "1")
}

async fn create_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
    owner: Option<&str>,
) -> crate::Result<()> {
    let mut command = vec![
        "createdb".to_owned(),
        "-U".to_owned(),
        credentials.username.clone(),
    ];
    if let Some(owner) = owner {
        command.push("-O".to_owned());
        command.push(owner.to_owned());
    }
    command.push(database.to_owned());
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            command,
        )
        .await
        .map(|_| ())
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
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "psql".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                "-d".to_owned(),
                "postgres".to_owned(),
                "-c".to_owned(),
                sql.to_owned(),
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error_value| {
            crate::GumgumError::structured(crate::Subsystem::Setup, crate::ErrorCode::Io, error)
                .likely_cause(error_value.to_report().message)
                .build()
        })
}

async fn drop_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
) -> crate::Result<()> {
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "dropdb".to_owned(),
                "--if-exists".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                database.to_owned(),
            ],
        )
        .await
        .map(|_| ())
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
