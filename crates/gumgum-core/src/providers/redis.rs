use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use crate::{Capability, sanitize_name};

use super::types::{ObjectProviderPlan, ProviderCredentials, ProviderSpec};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Kv,
        provider: "redis.main".to_owned(),
        container: "gumgum-provider-redis-main".to_owned(),
        image: "redis:7-alpine".to_owned(),
        port: 6379,
        protocol: "redis".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure redis.main provider is running".to_owned(),
        format!("reserve Redis key prefix {safe_name}:"),
        format!("publish DNS {dns} to redis.main"),
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("redis-cli -u redis://{dns}:6379/0"),
        format!("RedisInsight host={dns} port=6379 database=0"),
    ]
}

pub(crate) async fn ensure_object(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
    let mut actions = ensure(&plan.provider).await?;
    let prefix = sanitize_name(&plan.name);
    actions.push(format!("reserved Redis key prefix {prefix}:"));
    actions.push(format!("published DNS {} to redis.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete_object(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
    let mut actions = ensure(&plan.provider).await?;
    let prefix = sanitize_name(&plan.name);
    actions.push(format!("released Redis key prefix {prefix}:"));
    actions.push(format!("removed DNS {} from redis.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn ensure(provider: &ProviderSpec) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start redis provider").await;
    }
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&provider.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg("gumgum-network")
            .arg(&provider.image),
        "could not create redis provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

pub(crate) async fn ensure_with_credentials(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start redis provider").await;
    }
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&provider.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg("gumgum-network")
            .arg(&provider.image)
            .arg("redis-server")
            .arg("--requirepass")
            .arg(credentials.password),
        "could not create redis provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_object_plan_actions_are_namespace_scoped() {
        let plan = crate::providers::object_provider_plan(
            Capability::Kv,
            "user-counters",
            "user-counters.kv.leostera.dev",
        );

        assert_eq!(plan.provider.provider, "redis.main");
        assert!(
            plan.actions
                .contains(&"reserve Redis key prefix user-counters:".to_owned())
        );
    }
}
