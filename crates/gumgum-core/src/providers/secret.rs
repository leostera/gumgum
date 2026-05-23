use crate::{Capability, sanitize_name};

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

pub(crate) fn actions(safe_name: &str, _dns: &str) -> Vec<String> {
    vec![
        "configure the platform secret provider secrets.platform".to_owned(),
        format!("map secret {safe_name} from the configured secret provider"),
        "do not materialize secret values in the graph".to_owned(),
    ]
}

pub(crate) fn connection_examples(name: &str, _dns: &str) -> Vec<String> {
    vec![
        format!("bw get item {name}"),
        format!("bitwarden://gumgum/{name}"),
    ]
}

pub(crate) fn provider_actions(plan: &ObjectProviderPlan) -> Vec<String> {
    vec![
        "secret provider is external; no secret value stored in GumGum graph".to_owned(),
        format!(
            "mapped secret {} through {}",
            sanitize_name(&plan.name),
            plan.provider.provider
        ),
        "configure Vaultwarden or 1Password credentials before runtime resolution".to_owned(),
    ]
}
