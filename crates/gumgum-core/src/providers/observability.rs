use crate::Capability;

use super::types::ProviderSpec;

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Observability,
        provider: "otel.platform".to_owned(),
        container: "gumgum-provider-otel-platform".to_owned(),
        image: "otel/opentelemetry-collector-contrib:latest".to_owned(),
        port: 4317,
        protocol: "otlp".to_owned(),
    }
}

pub(crate) fn actions(_safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure otel.platform provider is running".to_owned(),
        format!("publish DNS {dns} to otel.platform"),
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<String> {
    vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")]
}
