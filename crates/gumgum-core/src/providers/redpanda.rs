use crate::{Capability, sanitize_name};
use tokio::process::Command as TokioCommand;

use super::docker::{
    create_provider_container, ensure_network, inspect, run_provider_command, start_existing,
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
    create_provider_container(
        provider,
        Vec::new(),
        vec![
            "redpanda".to_owned(),
            "start".to_owned(),
            "--overprovisioned".to_owned(),
            "--smp".to_owned(),
            "1".to_owned(),
            "--memory".to_owned(),
            "512M".to_owned(),
            "--reserve-memory".to_owned(),
            "0M".to_owned(),
            "--node-id".to_owned(),
            "0".to_owned(),
            "--check=false".to_owned(),
            "--kafka-addr".to_owned(),
            "0.0.0.0:9092".to_owned(),
            "--advertise-kafka-addr".to_owned(),
            "gumgum-provider-redpanda-main:9092".to_owned(),
        ],
    )
    .await
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
