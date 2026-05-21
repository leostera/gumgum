use crate::Capability;
use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ProviderConfig, ProviderSpec, ProviderStatus};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Secret,
        provider: "vaultwarden.main".to_owned(),
        container: "gumgum-provider-vaultwarden-main".to_owned(),
        image: "vaultwarden/server:latest".to_owned(),
        port: 80,
        protocol: "bitwarden-compatible".to_owned(),
    }
}

pub(crate) fn handles_config(config: &ProviderConfig) -> bool {
    config.capability == Capability::Secret
        && matches!(config.kind.as_str(), "vaultwarden" | "bitwarden")
}

pub fn actions(safe_name: &str, _dns: &str) -> Vec<String> {
    vec![
        "ensure vaultwarden.main provider is running".to_owned(),
        format!("map secret {safe_name} through vaultwarden.main"),
        "do not materialize secret values in the graph".to_owned(),
    ]
}

pub fn connection_examples(name: &str, _dns: &str) -> Vec<String> {
    vec![
        format!("bw get item {name}"),
        format!("bitwarden://gumgum/{name}"),
    ]
}

pub(crate) async fn ensure() -> crate::Result<Vec<String>> {
    let provider = spec();
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(&provider, "could not start vaultwarden provider").await;
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
            .arg(&provider.image),
        "could not create vaultwarden provider",
    )
    .await?;
    Ok(created_provider_actions(&provider))
}

pub(crate) async fn status() -> ProviderStatus {
    let provider = spec();
    let running = super::docker::running(&provider.container).await;
    ProviderStatus {
        capability: Capability::Secret,
        provider: provider.provider,
        container: provider.container,
        image: provider.image,
        port: provider.port,
        running,
    }
}
