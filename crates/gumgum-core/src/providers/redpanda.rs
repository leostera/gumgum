use crate::Capability;

use super::types::ProviderSpec;

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Queue,
        provider: "redpanda.main".to_owned(),
        container: "gumgum-provider-redpanda-main".to_owned(),
        image: "redpandadata/redpanda:latest".to_owned(),
        port: 9092,
        protocol: "kafka".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure redpanda.main provider is running".to_owned(),
        format!("ensure topic {safe_name} exists"),
        format!("publish DNS {dns} to redpanda.main"),
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("kcat -b {dns}:9092 -t {name}"),
        format!("KAFKA_BROKERS={dns}:9092 KAFKA_TOPIC={name}"),
    ]
}
