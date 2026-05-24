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

fn provider_environment_label(provider: &ProviderSpec) -> Option<String> {
    provider
        .provider
        .rsplit_once('.')
        .map(|(_, env)| env)
        .filter(|env| *env != "main" && *env != "platform")
        .map(str::to_owned)
}

pub(crate) fn provider_binds(provider: &ProviderSpec) -> Vec<String> {
    let env = provider
        .provider
        .rsplit_once('.')
        .map(|(_, env)| env)
        .unwrap_or("main");
    let kind = match provider.capability {
        crate::Capability::Db => Some(("postgres", "/var/lib/postgresql/data")),
        crate::Capability::Kv => Some(("redis", "/data")),
        crate::Capability::Blob => Some(("minio", "/data")),
        crate::Capability::Queue => Some(("redpanda", "/var/lib/redpanda/data")),
        _ => None,
    };
    kind.map(|(kind, mount)| format!("/gumgum/volumes/providers/{env}/{kind}:{mount}"))
        .into_iter()
        .collect()
}

fn provider_bind_fingerprint(provider: &ProviderSpec) -> String {
    provider_binds(provider).join("|")
}

pub(crate) async fn provider_needs_recreate(provider: &ProviderSpec) -> bool {
    let Ok(docker) = DockerEngine::local() else {
        return false;
    };
    match docker.inspect_container(&provider.container).await {
        Ok(Some(snapshot)) => {
            snapshot
                .labels
                .get("gumgum.provider.binds")
                .map(String::as_str)
                != Some(provider_bind_fingerprint(provider).as_str())
        }
        _ => false,
    }
}

pub(crate) async fn create_provider_container(
    provider: &ProviderSpec,
    env: Vec<(String, String)>,
    command: Vec<String>,
) -> crate::Result<Vec<String>> {
    let docker = DockerEngine::local()?;
    docker.pull_image(&provider.image).await?;
    let mut labels = HashMap::from([
        ("gumgum.managed".to_owned(), "provider".to_owned()),
        (
            "gumgum.provider.binds".to_owned(),
            provider_bind_fingerprint(provider),
        ),
    ]);
    if let Some(env) = provider_environment_label(provider) {
        labels.insert("gumgum.environment".to_owned(), env);
    }
    docker
        .create_and_start_container(ContainerRunSpec {
            name: provider.container.clone(),
            image: provider.image.clone(),
            network: GUMGUM_NETWORK.to_owned(),
            restart_unless_stopped: true,
            labels,
            env,
            binds: provider_binds(provider),
            ports: Vec::new(),
            command,
            entrypoint: Vec::new(),
        })
        .await?;
    Ok(created_provider_actions(provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Capability;

    #[test]
    fn provider_environment_label_skips_main_and_keeps_envs() {
        let mut provider = ProviderSpec {
            capability: Capability::Kv,
            provider: "redis.main".to_owned(),
            container: "gumgum-provider-redis-main".to_owned(),
            image: "redis:7-alpine".to_owned(),
            port: 6379,
            protocol: "redis".to_owned(),
        };
        assert_eq!(provider_environment_label(&provider), None);

        provider.provider = "redis.prod".to_owned();
        assert_eq!(
            provider_environment_label(&provider).as_deref(),
            Some("prod")
        );
    }

    #[test]
    fn provider_binds_make_stateful_providers_durable() {
        let cases = [
            (
                Capability::Db,
                "gumgum-prod-provider-postgres",
                "postgres",
                "/var/lib/postgresql/data",
            ),
            (
                Capability::Kv,
                "gumgum-prod-provider-redis",
                "redis",
                "/data",
            ),
            (
                Capability::Blob,
                "gumgum-prod-provider-minio",
                "minio",
                "/data",
            ),
            (
                Capability::Queue,
                "gumgum-prod-provider-redpanda",
                "redpanda",
                "/var/lib/redpanda/data",
            ),
        ];

        for (capability, container, kind, mount) in cases {
            let provider = ProviderSpec {
                capability,
                provider: format!("{}.prod", capability.provider()),
                container: container.to_owned(),
                image: "example:latest".to_owned(),
                port: 1234,
                protocol: "tcp".to_owned(),
            };
            assert_eq!(
                provider_binds(&provider),
                vec![format!("/gumgum/volumes/providers/prod/{kind}:{mount}")]
            );
        }
    }
}
