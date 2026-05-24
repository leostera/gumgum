use crate::{Capability, CoreAction, CoreActions, sanitize_name};

use super::types::{ObjectProviderPlan, ProviderSpec};

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

pub(crate) fn actions(safe_name: &str, _dns: &str) -> CoreActions {
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

pub(crate) fn connection_examples(name: &str, _dns: &str) -> Vec<String> {
    vec![
        format!("bw get item {name}"),
        format!("bitwarden://gumgum/{name}"),
    ]
}

pub(crate) fn provider_actions(plan: &ObjectProviderPlan) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Secret,
            provider: plan.provider.provider.clone(),
        },
        CoreAction::ProviderObjectDesiredRemoved {
            capability: Capability::Secret,
            name: sanitize_name(&plan.name),
        },
    ]
}
