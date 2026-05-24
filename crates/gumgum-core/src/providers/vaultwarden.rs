use crate::{Capability, ContainerRunSpec, CoreAction, CoreActions, DockerEngine};
use std::collections::HashMap;

use super::docker::{ensure_network, start_existing};
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

pub fn actions(safe_name: &str, _dns: &str) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Secret,
            provider: "secrets.platform".to_owned(),
        },
        CoreAction::ProviderObjectDesiredRemoved {
            capability: Capability::Secret,
            name: safe_name.to_owned(),
        },
    ]
}

pub fn connection_examples(name: &str, _dns: &str) -> Vec<crate::ConnectionExample> {
    vec![
        crate::ConnectionExample::BitwardenCli { name: name.to_owned() },
        crate::ConnectionExample::BitwardenUri { name: name.to_owned() },
    ]
}

pub(crate) async fn ensure() -> crate::Result<CoreActions> {
    let provider = spec();
    ensure_network().await?;
    if let Some(existing) = DockerEngine::local()?
        .inspect_container(&provider.container)
        .await?
    {
        if existing
            .labels
            .get("gumgum.platform.fingerprint")
            .map(String::as_str)
            == Some("v3")
        {
            return start_existing(&provider, "could not start vaultwarden provider").await;
        }
        DockerEngine::local()?
            .remove_container_force(&provider.container)
            .await?;
    }
    let docker = DockerEngine::local()?;
    docker.pull_image(&provider.image).await?;
    docker
        .create_and_start_container(ContainerRunSpec {
            name: provider.container.clone(),
            image: provider.image.clone(),
            network: "gumgum-network".to_owned(),
            restart_unless_stopped: true,
            labels: HashMap::from([
                ("gumgum.managed".to_owned(), "platform".to_owned()),
                (
                    "gumgum.platform.service".to_owned(),
                    "vaultwarden".to_owned(),
                ),
                ("gumgum.capability".to_owned(), "secret".to_owned()),
                ("gumgum.platform.fingerprint".to_owned(), "v3".to_owned()),
            ]),
            env: vec![
                ("SIGNUPS_ALLOWED".to_owned(), "false".to_owned()),
                ("WEBSOCKET_ENABLED".to_owned(), "true".to_owned()),
            ],
            binds: vec!["/gumgum/volumes/platform/vaultwarden:/data".to_owned()],
            ports: Vec::new(),
            command: Vec::new(),
            entrypoint: Vec::new(),
        })
        .await?;
    Ok(vec![CoreAction::PlatformSecretServiceCreated {
        provider: provider.provider,
        container: provider.container,
    }])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vaultwarden_spec_is_platform_singleton() {
        let spec = spec();
        assert_eq!(spec.provider, "secrets.platform");
        assert_eq!(spec.container, "gumgum-vaultwarden");
    }
}
