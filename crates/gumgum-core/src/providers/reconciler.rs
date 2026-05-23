use crate::Capability;
#[cfg(test)]
use crate::{ErrorCode, GumgumError, Subsystem};

use super::types::{ObjectProviderPlan, ProviderConfig, ProviderCredentials, ProviderStatus};

pub struct ProviderReconciler;

impl ProviderReconciler {
    pub async fn ensure(plan: &ObjectProviderPlan) -> crate::Result<Vec<String>> {
        Self::ensure_with_credentials(plan, None).await
    }

    pub async fn ensure_with_credentials(
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<Vec<String>> {
        match plan.capability {
            Capability::Db => {
                super::postgres::ensure_object(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::postgres_local_dev),
                )
                .await
            }
            Capability::Kv => {
                super::redis::ensure_object(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::redis_local_dev),
                )
                .await
            }
            Capability::Blob => {
                super::minio::ensure(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::minio_local_dev),
                )
                .await
            }
            Capability::Queue => super::redpanda::ensure(plan).await,
            Capability::Secret => Ok(super::secret::provider_actions(plan)),
            _ => Ok(plan.actions.clone()),
        }
    }

    pub async fn delete_with_credentials(
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<Vec<String>> {
        match plan.capability {
            Capability::Db => {
                super::postgres::delete_object(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::postgres_local_dev),
                )
                .await
            }
            Capability::Kv => super::redis::delete_object(plan).await,
            Capability::Blob => {
                super::minio::delete(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::minio_local_dev),
                )
                .await
            }
            Capability::Queue => super::redpanda::delete(plan).await,
            _ => Ok(vec![format!(
                "removed desired {} object {}; provider cleanup is not implemented yet",
                plan.capability, plan.name
            )]),
        }
    }

    pub async fn boot_defaults(
        _credentials: &[(String, ProviderCredentials)],
        root_domain: &str,
    ) -> crate::Result<Vec<String>> {
        let mut actions = Vec::new();
        actions.extend(super::vaultwarden::ensure().await?);
        actions.extend(super::observability::ensure_platform_stack(root_domain).await?);
        Ok(actions)
    }

    pub async fn ensure_configured_provider(config: &ProviderConfig) -> crate::Result<Vec<String>> {
        if super::vaultwarden::handles_config(config) {
            return super::vaultwarden::ensure().await;
        }
        Ok(vec![format!(
            "configured {} provider {}",
            config.capability, config.provider
        )])
    }

    pub async fn statuses() -> Vec<ProviderStatus> {
        let mut statuses = Vec::new();
        for capability in [
            Capability::Db,
            Capability::Kv,
            Capability::Blob,
            Capability::Queue,
        ] {
            let spec = super::specs::provider_spec(capability);
            let running = super::docker::running(&spec.container).await;
            statuses.push(ProviderStatus {
                capability,
                provider: spec.provider,
                container: spec.container,
                image: spec.image,
                port: spec.port,
                running,
            });
        }
        statuses.push(super::vaultwarden::status().await);
        statuses.extend(super::observability::platform_statuses().await);
        statuses
    }
}

#[cfg(test)]
pub(crate) fn provider_credentials(
    credentials: &[(String, ProviderCredentials)],
    provider: &str,
) -> crate::Result<ProviderCredentials> {
    credentials
        .iter()
        .find(|(name, _)| name == provider)
        .map(|(_, credentials)| credentials.clone())
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                format!("missing credentials for {provider}"),
            )
            .build()
        })
}
