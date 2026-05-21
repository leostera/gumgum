use crate::{Capability, sanitize_name};
use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ObjectProviderPlan, ProviderSpec};

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

pub(crate) async fn ensure(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider).await?;
    let topic = sanitize_name(&plan.name);
    ensure_topic(&topic).await?;
    actions.push(format!("ensured topic {topic} on {}", provider.provider));
    actions.push(format!("published DNS {} to redpanda.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider).await?;
    let topic = sanitize_name(&plan.name);
    delete_topic(&topic).await?;
    actions.push(format!("deleted topic {topic} from {}", provider.provider));
    actions.push(format!("removed DNS {} from redpanda.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn ensure_provider(provider: &ProviderSpec) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start redpanda provider").await;
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
            .arg("redpanda")
            .arg("start")
            .arg("--overprovisioned")
            .arg("--smp")
            .arg("1")
            .arg("--memory")
            .arg("512M")
            .arg("--reserve-memory")
            .arg("0M")
            .arg("--node-id")
            .arg("0")
            .arg("--check=false")
            .arg("--kafka-addr")
            .arg("0.0.0.0:9092")
            .arg("--advertise-kafka-addr")
            .arg("gumgum-provider-redpanda-main:9092"),
        "could not create redpanda provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

async fn ensure_topic(topic: &str) -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("exec")
            .arg("gumgum-provider-redpanda-main")
            .arg("rpk")
            .arg("topic")
            .arg("create")
            .arg(topic)
            .arg("--if-not-exists"),
        "could not ensure redpanda topic",
    )
    .await
}

async fn delete_topic(topic: &str) -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("exec")
            .arg("gumgum-provider-redpanda-main")
            .arg("rpk")
            .arg("topic")
            .arg("delete")
            .arg(topic),
        "could not delete redpanda topic",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redpanda_object_plan_actions_are_topic_scoped() {
        let plan = crate::providers::object_provider_plan(
            Capability::Queue,
            "visit-events",
            "visit-events.queue.leostera.dev",
        );

        assert_eq!(plan.provider.provider, "redpanda.main");
        assert!(
            plan.actions
                .contains(&"ensure topic visit-events exists".to_owned())
        );
    }
}
