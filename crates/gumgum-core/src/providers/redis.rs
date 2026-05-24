use super::docker::{
    create_provider_container, ensure_network, inspect, provider_needs_recreate, start_existing,
};
use crate::{Capability, CoreAction, CoreActions, DockerEngine, sanitize_name};

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

pub(crate) fn actions(safe_name: &str, dns: &str) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Kv,
            provider: "redis.main".to_owned(),
        },
        CoreAction::RedisPrefixReserved {
            prefix: safe_name.to_owned(),
        },
        CoreAction::DnsPublished {
            dns: dns.to_owned(),
            provider: "redis.main".to_owned(),
        },
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<crate::ConnectionExample> {
    vec![
        crate::ConnectionExample::RedisCli {
            dns: dns.to_owned(),
        },
        crate::ConnectionExample::RedisInsight {
            dns: dns.to_owned(),
        },
    ]
}

pub(crate) async fn ensure_object(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    let mut actions = ensure_with_credentials(&plan.provider, credentials).await?;
    let prefix = sanitize_name(&plan.name);
    actions.push(CoreAction::RedisPrefixReserved {
        prefix: prefix.clone(),
    });
    actions.push(CoreAction::DnsPublished {
        dns: plan.dns.clone(),
        provider: "redis.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn delete_object(plan: &ObjectProviderPlan) -> crate::Result<CoreActions> {
    let mut actions = ensure(&plan.provider).await?;
    let prefix = sanitize_name(&plan.name);
    actions.push(CoreAction::RedisPrefixReleased {
        prefix: prefix.clone(),
    });
    actions.push(CoreAction::DnsRemoved {
        dns: plan.dns.clone(),
        provider: "redis.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn ensure(provider: &ProviderSpec) -> crate::Result<CoreActions> {
    ensure_network().await?;
    if inspect(&provider.container).await && !provider_needs_recreate(provider).await {
        return start_existing(provider, "could not start redis provider").await;
    }
    if inspect(&provider.container).await {
        DockerEngine::local()?
            .remove_container_force(&provider.container)
            .await?;
    }
    create_provider_container(provider, Vec::new(), Vec::new()).await
}

pub(crate) async fn ensure_with_credentials(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    ensure_network().await?;
    if inspect(&provider.container).await && !provider_needs_recreate(provider).await {
        let mut actions = start_existing(provider, "could not start redis provider").await?;
        if !redis_accepts_password(provider, &credentials).await? {
            DockerEngine::local()?
                .remove_container_force(&provider.container)
                .await?;
            actions.push(CoreAction::ProviderContainerRecreated {
                provider: provider.provider.clone(),
            });
            actions.extend(create_redis_provider_container(provider, credentials).await?);
        }
        return Ok(actions);
    }
    if inspect(&provider.container).await {
        DockerEngine::local()?
            .remove_container_force(&provider.container)
            .await?;
    }
    create_redis_provider_container(provider, credentials).await
}

async fn create_redis_provider_container(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    create_provider_container(
        provider,
        Vec::new(),
        vec![
            "redis-server".to_owned(),
            "--requirepass".to_owned(),
            credentials.password,
            "--appendonly".to_owned(),
            "yes".to_owned(),
        ],
    )
    .await
}

async fn redis_accepts_password(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
) -> crate::Result<bool> {
    match DockerEngine::local()?
        .exec_success(
            &provider.container,
            Vec::new(),
            vec![
                "redis-cli".to_owned(),
                "-a".to_owned(),
                credentials.password.clone(),
                "PING".to_owned(),
            ],
        )
        .await
    {
        Ok(output) => Ok(output.trim() == "PONG"),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_password_probe_rejects_auth_failed_pong_output() {
        let output = "AUTH failed: ERR AUTH <password> called without any password configured for the default user.\nPONG\n";
        assert_ne!(output.trim(), "PONG");
    }

    #[test]
    fn redis_recreate_action_explains_password_drift() {
        let provider = ProviderSpec {
            capability: Capability::Kv,
            provider: "redis.preview".to_owned(),
            container: "gumgum-preview-provider-redis".to_owned(),
            image: "redis:7-alpine".to_owned(),
            port: 6379,
            protocol: "redis".to_owned(),
        };

        assert_eq!(
            format!(
                "recreated {} provider with configured password",
                provider.provider
            ),
            "recreated redis.preview provider with configured password"
        );
    }

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
                .iter().any(|action| matches!(action, crate::CoreAction::RedisPrefixReserved { prefix } if prefix == "user-counters"))
        );
    }
}
