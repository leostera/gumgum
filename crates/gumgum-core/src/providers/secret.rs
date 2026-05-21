use crate::sanitize_name;

use super::types::ObjectProviderPlan;

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
