use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use crate::Capability;

use super::types::{ProviderCredentials, ProviderSpec};

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
