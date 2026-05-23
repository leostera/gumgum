use crate::Capability;

use super::docker::{create_provider_container, ensure_network, inspect, start_existing};
use super::types::{ProviderConfig, ProviderSpec, ProviderStatus};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Secret,
        provider: "secrets.platform".to_owned(),
        container: "gumgum-vaultwarden".to_owned(),
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
        "ensure secrets.platform provider is running".to_owned(),
        format!("map secret {safe_name} through secrets.platform"),
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
    create_provider_container(&provider, Vec::new(), Vec::new()).await
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
