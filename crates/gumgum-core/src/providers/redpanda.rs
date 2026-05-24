use crate::{Capability, CoreAction, CoreActions, DockerEngine, sanitize_name};

use super::docker::{
    create_provider_container, ensure_network, inspect, provider_needs_recreate, start_existing,
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

pub(crate) fn actions(safe_name: &str, dns: &str) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Queue,
            provider: "redpanda.main".to_owned(),
        },
        CoreAction::QueueTopicEnsured {
            topic: safe_name.to_owned(),
            provider: "redpanda.main".to_owned(),
        },
        CoreAction::DnsPublished {
            dns: dns.to_owned(),
            provider: "redpanda.main".to_owned(),
        },
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("kcat -b {dns}:9092 -t {name}"),
        format!("KAFKA_BROKERS={dns}:9092 KAFKA_TOPIC={name}"),
    ]
}

pub(crate) async fn ensure(plan: &ObjectProviderPlan) -> crate::Result<CoreActions> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider).await?;
    let topic = sanitize_name(&plan.name);
    ensure_topic(provider, &topic).await?;
    actions.push(CoreAction::QueueTopicEnsured {
        topic: topic.clone(),
        provider: provider.provider.clone(),
    });
    actions.push(CoreAction::DnsPublished {
        dns: plan.dns.clone(),
        provider: "redpanda.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn delete(plan: &ObjectProviderPlan) -> crate::Result<CoreActions> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider).await?;
    let topic = sanitize_name(&plan.name);
    delete_topic(provider, &topic).await?;
    actions.push(CoreAction::QueueTopicDeleted {
        topic: topic.clone(),
        provider: provider.provider.clone(),
    });
    actions.push(CoreAction::DnsRemoved {
        dns: plan.dns.clone(),
        provider: "redpanda.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn ensure_provider(provider: &ProviderSpec) -> crate::Result<CoreActions> {
    ensure_network().await?;
    if inspect(&provider.container).await && !provider_needs_recreate(provider).await {
        return start_existing(provider, "could not start redpanda provider").await;
    }
    if inspect(&provider.container).await {
        DockerEngine::local()?
            .remove_container_force(&provider.container)
            .await?;
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
                .iter().any(|action| matches!(action, crate::CoreAction::QueueTopicEnsured { topic, .. } if topic == "visit-events"))
        );
    }
}
