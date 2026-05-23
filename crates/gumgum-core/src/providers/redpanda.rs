use crate::{Capability, DockerEngine, sanitize_name};

use super::docker::{create_provider_container, ensure_network, inspect, start_existing};
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
    ensure_topic(provider, &topic).await?;
    actions.push(format!("ensured topic {topic} on {}", provider.provider));
    actions.push(format!("published DNS {} to redpanda.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider).await?;
    let topic = sanitize_name(&plan.name);
    delete_topic(provider, &topic).await?;
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
            format!("{}:9092", provider.container),
        ],
    )
    .await
}

async fn ensure_topic(provider: &ProviderSpec, topic: &str) -> crate::Result<()> {
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            Vec::new(),
            vec![
                "rpk".to_owned(),
                "topic".to_owned(),
                "create".to_owned(),
                topic.to_owned(),
                "--if-not-exists".to_owned(),
            ],
        )
        .await
        .map(|_| ())
}

async fn delete_topic(provider: &ProviderSpec, topic: &str) -> crate::Result<()> {
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            Vec::new(),
            vec![
                "rpk".to_owned(),
                "topic".to_owned(),
                "delete".to_owned(),
                topic.to_owned(),
            ],
        )
        .await
        .map(|_| ())
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
