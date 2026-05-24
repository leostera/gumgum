use crate::{Capability, CoreAction, CoreActions, sanitize_name};

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
    let safe_name = sanitize_name(name);
    let provider = object_provider_spec(capability, &safe_name);
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

pub fn connection_examples(
    capability: Capability,
    name: &str,
    dns: &str,
) -> Vec<crate::ConnectionExample> {
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

fn object_provider_spec(capability: Capability, safe_name: &str) -> ProviderSpec {
    let provider = provider_spec(capability);
    let Some(env) = object_environment(safe_name) else {
        return provider;
    };
    let env = sanitize_name(env);
    let provider_name = provider
        .provider
        .rsplit_once('.')
        .map(|(prefix, _)| format!("{prefix}.{env}"))
        .unwrap_or_else(|| format!("{}.{}", provider.provider, env));
    let container_suffix = provider
        .container
        .strip_prefix("gumgum-")
        .unwrap_or(&provider.container)
        .strip_suffix("-main")
        .unwrap_or_else(|| {
            provider
                .container
                .strip_prefix("gumgum-")
                .unwrap_or(&provider.container)
        });
    let container = format!("gumgum-{env}-{container_suffix}");
    ProviderSpec {
        provider: provider_name,
        container,
        ..provider
    }
}

fn object_environment(safe_name: &str) -> Option<&str> {
    safe_name
        .strip_suffix("-preview")
        .map(|_| "preview")
        .or_else(|| safe_name.strip_suffix("-prod").map(|_| "prod"))
}

fn provider_actions(capability: Capability, safe_name: &str, dns: &str) -> CoreActions {
    match capability {
        Capability::Db => super::postgres::actions(safe_name, dns),
        Capability::Kv => super::redis::actions(safe_name, dns),
        Capability::Blob => super::minio::actions(safe_name, dns),
        Capability::Queue => super::redpanda::actions(safe_name, dns),
        Capability::Secret => super::secret::actions(safe_name, dns),
        Capability::Observability => super::observability::actions(safe_name, dns),
        Capability::Manual => vec![CoreAction::ProviderConfigured {
            capability,
            provider: capability.provider().to_owned(),
        }],
    }
}
