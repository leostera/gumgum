use crate::{Capability, sanitize_name};

use super::types::{ObjectProviderPlan, ProviderSpec};

pub fn provider_spec(capability: Capability) -> ProviderSpec {
    match capability {
        Capability::Db => super::postgres::spec(),
        Capability::Kv => super::redis::spec(),
        Capability::Blob => super::minio::spec(),
        Capability::Queue => super::redpanda::spec(),
        Capability::Secret => super::secret::spec(),
        Capability::Observability => super::observability::spec(),
        Capability::Manual => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-manual-main".to_owned(),
            image: "manual".to_owned(),
            port: 0,
            protocol: "manual".to_owned(),
        },
    }
}

pub fn object_provider_plan(capability: Capability, name: &str, dns: &str) -> ObjectProviderPlan {
    let provider = provider_spec(capability);
    let safe_name = sanitize_name(name);
    ObjectProviderPlan {
        capability,
        name: name.to_owned(),
        dns: dns.to_owned(),
        actions: provider_actions(capability, &safe_name, dns),
        connection_examples: connection_examples(capability, name, dns),
        provider,
        object_password: None,
    }
}

pub fn connection_examples(capability: Capability, name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => super::postgres::connection_examples(name, dns),
        Capability::Kv => super::redis::connection_examples(name, dns),
        Capability::Blob => super::minio::connection_examples(name, dns),
        Capability::Queue => super::redpanda::connection_examples(name, dns),
        Capability::Secret => super::secret::connection_examples(name, dns),
        Capability::Observability => super::observability::connection_examples(name, dns),
        Capability::Manual => Vec::new(),
    }
}

fn provider_actions(capability: Capability, safe_name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => super::postgres::actions(safe_name, dns),
        Capability::Kv => super::redis::actions(safe_name, dns),
        Capability::Blob => super::minio::actions(safe_name, dns),
        Capability::Queue => super::redpanda::actions(safe_name, dns),
        Capability::Secret => super::secret::actions(safe_name, dns),
        Capability::Observability => super::observability::actions(safe_name, dns),
        Capability::Manual => {
            vec!["manual provider requires operator-managed backing service".to_owned()]
        }
    }
}
