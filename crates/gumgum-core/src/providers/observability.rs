use crate::{Capability, ContainerRunSpec, DockerEngine};
use std::collections::HashMap;

use super::types::{ProviderSpec, ProviderStatus};

const GUMGUM_NETWORK: &str = "gumgum-network";

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Observability,
        provider: "observability.platform".to_owned(),
        container: "gumgum-otel".to_owned(),
        image: "otel/opentelemetry-collector-contrib:latest".to_owned(),
        port: 4317,
        protocol: "otlp".to_owned(),
    }
}

pub(crate) fn actions(_safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure observability.platform provider is running".to_owned(),
        format!("publish DNS {dns} to observability.platform"),
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<String> {
    vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")]
}

pub(crate) async fn ensure_platform_stack(root_domain: &str) -> crate::Result<Vec<String>> {
    let mut actions = Vec::new();
    for provider in platform_specs(root_domain) {
        actions.extend(ensure_platform_container(&provider).await?);
    }
    Ok(actions)
}

async fn ensure_platform_container(provider: &ProviderSpec) -> crate::Result<Vec<String>> {
    let docker = DockerEngine::local()?;
    docker.ensure_network(GUMGUM_NETWORK).await?;
    if docker
        .inspect_container(&provider.container)
        .await?
        .is_some()
    {
        docker.start_container(&provider.container).await?;
        return Ok(vec![format!("started existing {}", provider.container)]);
    }
    docker.pull_image(&provider.image).await?;
    docker
        .create_and_start_container(ContainerRunSpec {
            name: provider.container.clone(),
            image: provider.image.clone(),
            network: GUMGUM_NETWORK.to_owned(),
            restart_unless_stopped: true,
            labels: platform_labels(provider),
            env: platform_env(provider),
            binds: Vec::new(),
            ports: Vec::new(),
            command: platform_command(provider),
            entrypoint: Vec::new(),
        })
        .await?;
    Ok(vec![format!(
        "created platform service {} ({})",
        provider.container, provider.provider
    )])
}

fn platform_labels(provider: &ProviderSpec) -> HashMap<String, String> {
    HashMap::from([
        ("gumgum.managed".to_owned(), "platform".to_owned()),
        (
            "gumgum.platform.service".to_owned(),
            provider.container.trim_start_matches("gumgum-").to_owned(),
        ),
        ("gumgum.capability".to_owned(), "observability".to_owned()),
    ])
}

fn platform_env(provider: &ProviderSpec) -> Vec<(String, String)> {
    if provider.container == "gumgum-grafana" {
        vec![
            ("GF_USERS_ALLOW_SIGN_UP".to_owned(), "false".to_owned()),
            ("GF_SECURITY_ADMIN_USER".to_owned(), "gumgum".to_owned()),
            (
                "GF_SECURITY_ADMIN_PASSWORD".to_owned(),
                std::env::var("GUMGUM_GRAFANA_ADMIN_PASSWORD")
                    .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
            ),
            (
                "GF_SERVER_ROOT_URL".to_owned(),
                "% (protocol)s://%(domain)s/".to_owned(),
            ),
        ]
    } else {
        Vec::new()
    }
}

fn platform_command(provider: &ProviderSpec) -> Vec<String> {
    match provider.container.as_str() {
        "gumgum-loki" => vec!["-config.file=/etc/loki/local-config.yaml".to_owned()],
        "gumgum-tempo" => vec!["-config.file=/etc/tempo.yaml".to_owned()],
        _ => Vec::new(),
    }
}

pub(crate) async fn platform_statuses() -> Vec<ProviderStatus> {
    let mut statuses = Vec::new();
    for provider in platform_specs("example.invalid") {
        let running = super::docker::running(&provider.container).await;
        statuses.push(ProviderStatus {
            capability: Capability::Observability,
            provider: provider.provider,
            container: provider.container,
            image: provider.image,
            port: provider.port,
            running,
        });
    }
    statuses
}

fn platform_specs(_root_domain: &str) -> Vec<ProviderSpec> {
    vec![
        spec(),
        ProviderSpec {
            capability: Capability::Observability,
            provider: "prometheus.platform".to_owned(),
            container: "gumgum-prometheus".to_owned(),
            image: "prom/prometheus:latest".to_owned(),
            port: 9090,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "grafana.platform".to_owned(),
            container: "gumgum-grafana".to_owned(),
            image: "grafana/grafana:latest".to_owned(),
            port: 3000,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "loki.platform".to_owned(),
            container: "gumgum-loki".to_owned(),
            image: "grafana/loki:latest".to_owned(),
            port: 3100,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "tempo.platform".to_owned(),
            container: "gumgum-tempo".to_owned(),
            image: "grafana/tempo:latest".to_owned(),
            port: 3200,
            protocol: "http".to_owned(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observability_platform_specs_use_singleton_container_names() {
        let specs = platform_specs("leostera.dev");
        assert!(
            specs
                .iter()
                .any(|spec| spec.container == "gumgum-prometheus")
        );
        assert!(specs.iter().any(|spec| spec.container == "gumgum-grafana"));
        assert!(specs.iter().any(|spec| spec.container == "gumgum-loki"));
        assert!(specs.iter().any(|spec| spec.container == "gumgum-tempo"));
        assert!(specs.iter().all(|spec| !spec.container.contains("preview")));
        assert!(specs.iter().all(|spec| !spec.container.contains("prod")));
    }

    #[test]
    fn grafana_platform_env_disables_signup_and_sets_admin() {
        let grafana = platform_specs("leostera.dev")
            .into_iter()
            .find(|spec| spec.container == "gumgum-grafana")
            .unwrap();
        let env = platform_env(&grafana);
        assert!(env.contains(&("GF_USERS_ALLOW_SIGN_UP".to_owned(), "false".to_owned())));
        assert!(
            env.iter()
                .any(|(name, _)| name == "GF_SECURITY_ADMIN_PASSWORD")
        );
    }

    #[test]
    fn platform_labels_mark_observability_singletons() {
        let labels = platform_labels(&spec());
        assert_eq!(
            labels.get("gumgum.managed").map(String::as_str),
            Some("platform")
        );
        assert_eq!(
            labels.get("gumgum.capability").map(String::as_str),
            Some("observability")
        );
    }
}
