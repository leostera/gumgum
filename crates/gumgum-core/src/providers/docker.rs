use std::collections::HashMap;

use crate::{ContainerRunSpec, DockerEngine};

use super::types::ProviderSpec;

const GUMGUM_NETWORK: &str = "gumgum-network";

pub(crate) async fn ensure_network() -> crate::Result<()> {
    DockerEngine::local()?
        .ensure_network(GUMGUM_NETWORK)
        .await?;
    Ok(())
}

pub(crate) async fn inspect(container: &str) -> bool {
    match DockerEngine::local() {
        Ok(docker) => docker
            .inspect_container(container)
            .await
            .ok()
            .flatten()
            .is_some(),
        Err(_) => false,
    }
}

pub(crate) async fn running(container: &str) -> bool {
    match DockerEngine::local() {
        Ok(docker) => docker.container_running(container).await.unwrap_or(false),
        Err(_) => false,
    }
}

pub(crate) fn created_provider_actions(provider: &ProviderSpec) -> Vec<String> {
    vec![format!(
        "created {} provider container {}",
        provider.provider, provider.container
    )]
}

pub(crate) async fn start_existing(
    provider: &ProviderSpec,
    _message: &str,
) -> crate::Result<Vec<String>> {
    DockerEngine::local()?
        .start_container(&provider.container)
        .await?;
    Ok(vec![format!(
        "started existing {} provider",
        provider.provider
    )])
}

pub(crate) async fn create_provider_container(
    provider: &ProviderSpec,
    env: Vec<(String, String)>,
    command: Vec<String>,
) -> crate::Result<Vec<String>> {
    let docker = DockerEngine::local()?;
    docker.pull_image(&provider.image).await?;
    docker
        .create_and_start_container(ContainerRunSpec {
            name: provider.container.clone(),
            image: provider.image.clone(),
            network: GUMGUM_NETWORK.to_owned(),
            restart_unless_stopped: true,
            labels: HashMap::from([("gumgum.managed".to_owned(), "provider".to_owned())]),
            env,
            binds: Vec::new(),
            ports: Vec::new(),
            command,
            entrypoint: Vec::new(),
        })
        .await?;
    Ok(created_provider_actions(provider))
}
