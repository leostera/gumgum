use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
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
    if database_exists(&plan.provider, &credentials, &database).await? {
        actions.push(format!("database {database} already exists"));
    } else {
        create_database(&plan.provider, &credentials, &database).await?;
        actions.push(format!("created database {database}"));
    }
    actions.push(format!("published DNS {} to postgres.main", plan.dns));
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
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&provider.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg("gumgum-network")
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.username_env, credentials.username
            ))
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.password_env, credentials.password
            ))
            .arg(&provider.image),
        "could not create postgres provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
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
) -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("exec")
            .arg("-e")
            .arg(format!("PGPASSWORD={}", credentials.password))
            .arg(&provider.container)
            .arg("createdb")
            .arg("-U")
            .arg(&credentials.username)
            .arg(database),
        "could not create postgres database",
    )
    .await
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
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
    fn postgres_sql_literal_escapes_single_quotes() {
        assert_eq!(shell_single_quote("team's-db"), "team''s-db");
    }
}
