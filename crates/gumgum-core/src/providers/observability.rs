use crate::Capability;

use super::types::{ProviderSpec, ProviderStatus};

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

pub(crate) async fn platform_statuses() -> Vec<ProviderStatus> {
    let mut statuses = Vec::new();
    for provider in platform_specs() {
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

fn platform_specs() -> Vec<ProviderSpec> {
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
        let specs = platform_specs();
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
}
