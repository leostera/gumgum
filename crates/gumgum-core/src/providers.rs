pub mod docker;
pub mod minio;
pub mod observability;
pub mod postgres;
pub mod reconciler;
pub mod redis;
pub mod redpanda;
pub mod secret;
pub mod specs;
pub mod types;
pub mod vaultwarden;

pub use reconciler::ProviderReconciler;
pub use specs::{connection_examples, object_provider_plan, provider_spec};
pub use types::{
    ObjectProviderPlan, ProviderConfig, ProviderCredentials, ProviderSpec, ProviderStatus,
    generated_secret_value,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Capability;

    #[test]
    fn provider_config_maps_secret_kinds_to_providers() {
        let local = ProviderConfig::new(Capability::Secret, "local", None, None);
        assert_eq!(local.provider, "local.secrets");
        let onepassword = ProviderConfig::new(
            Capability::Secret,
            "onepassword",
            Some("http://onepassword:8080".to_owned()),
            Some("GumGum".to_owned()),
        );
        assert_eq!(onepassword.provider, "onepassword.main");
        assert_eq!(onepassword.vault.as_deref(), Some("GumGum"));
        let vaultwarden = ProviderConfig::new(Capability::Secret, "vaultwarden", None, None);
        assert_eq!(vaultwarden.provider, "vaultwarden.main");
    }

    #[test]
    fn provider_specs_cover_core_capabilities() {
        let db = provider_spec(Capability::Db);
        assert_eq!(db.provider, "postgres.main");
        assert_eq!(db.container, "gumgum-provider-postgres-main");
        assert_eq!(db.port, 5432);

        let kv = provider_spec(Capability::Kv);
        assert_eq!(kv.provider, "redis.main");
        assert_eq!(kv.port, 6379);

        let blob = provider_spec(Capability::Blob);
        assert_eq!(blob.provider, "minio.main");
        assert_eq!(blob.protocol, "s3");

        let secret = provider_spec(Capability::Secret);
        assert_eq!(secret.provider, "onepassword.main");
        assert_eq!(secret.protocol, "onepassword-connect");
    }

    #[tokio::test]
    async fn provider_statuses_cover_inspectable_backends() {
        let statuses = ProviderReconciler::statuses().await;

        assert_eq!(statuses.len(), 7);
        assert!(
            statuses
                .iter()
                .any(|status| status.provider == "redis.main")
        );
        assert!(
            statuses
                .iter()
                .any(|status| status.provider == "vaultwarden.main")
        );
        assert!(
            statuses
                .iter()
                .any(|status| status.container == "gumgum-provider-postgres-main")
        );
    }

    #[test]
    fn db_provider_reconciler_is_scoped_to_postgres_container() {
        let plan = object_provider_plan(Capability::Db, "visits", "visits.db.example.test");

        assert_eq!(plan.provider.container, "gumgum-provider-postgres-main");
        assert_eq!(plan.provider.image, "postgres:16-alpine");
        assert_eq!(plan.provider.port, 5432);
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure postgres.main provider is running")
        );
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure database visits exists")
        );
    }

    #[test]
    fn kv_provider_reconciler_is_scoped_to_redis_container() {
        let plan = object_provider_plan(Capability::Kv, "sessions", "sessions.kv.example.test");

        assert_eq!(plan.provider.container, "gumgum-provider-redis-main");
        assert_eq!(plan.provider.image, "redis:7-alpine");
        assert_eq!(plan.provider.port, 6379);
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure redis.main provider is running")
        );
    }

    #[test]
    fn kv_provider_delete_plan_is_scoped_to_redis_namespace() {
        let plan = object_provider_plan(
            Capability::Kv,
            "user-counters",
            "user-counters.kv.example.test",
        );

        assert_eq!(plan.provider.provider, "redis.main");
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "reserve Redis key prefix user-counters:")
        );
    }

    #[test]
    fn provider_boot_requires_all_default_credentials() {
        let credentials = vec![(
            "redis.main".to_owned(),
            ProviderCredentials::generated("REDIS_USER", "REDIS_PASSWORD", "gumgum"),
        )];
        let error = reconciler::provider_credentials(&credentials, "postgres.main")
            .unwrap_err()
            .to_report();

        assert_eq!(error.message, "missing credentials for postgres.main");
    }

    #[test]
    fn provider_reconciler_accepts_explicit_credentials_for_bucket() {
        let credentials = ProviderCredentials {
            username_env: "MINIO_ROOT_USER".to_owned(),
            password_env: "MINIO_ROOT_PASSWORD".to_owned(),
            username: "gumgum".to_owned(),
            password: "secret".to_owned(),
        };
        let plan = object_provider_plan(Capability::Blob, "uploads", "uploads.blob.example.test");

        assert_eq!(credentials.username, "gumgum");
        assert_eq!(plan.provider.provider, "minio.main");
    }

    #[test]
    fn minio_credentials_have_named_env_keys() {
        let credentials = ProviderCredentials::minio_local_dev();

        assert_eq!(credentials.username_env, "MINIO_ROOT_USER");
        assert_eq!(credentials.password_env, "MINIO_ROOT_PASSWORD");
        assert!(!credentials.username.is_empty());
        assert!(!credentials.password.is_empty());
    }

    #[test]
    fn blob_provider_reconciler_is_scoped_to_minio_container() {
        let plan = object_provider_plan(Capability::Blob, "uploads", "uploads.blob.example.test");

        assert_eq!(plan.provider.container, "gumgum-provider-minio-main");
        assert_eq!(plan.provider.image, "minio/minio:latest");
        assert_eq!(plan.provider.port, 9000);
        assert_eq!(
            docker::created_provider_actions(&plan.provider),
            vec!["created minio.main provider container gumgum-provider-minio-main"]
        );
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure bucket uploads exists")
        );
    }

    #[test]
    fn queue_provider_reconciler_is_scoped_to_redpanda_topic() {
        let plan = object_provider_plan(
            Capability::Queue,
            "visit-events",
            "visit-events.queue.example.test",
        );

        assert_eq!(plan.provider.provider, "redpanda.main");
        assert_eq!(plan.provider.container, "gumgum-provider-redpanda-main");
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure topic visit-events exists")
        );
    }

    #[test]
    fn blob_provider_delete_plan_is_scoped_to_minio_bucket() {
        let plan = object_provider_plan(
            Capability::Blob,
            "visit-requests",
            "visit-requests.blob.example.test",
        );

        assert_eq!(plan.provider.provider, "minio.main");
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure bucket visit-requests exists")
        );
    }

    #[test]
    fn vaultwarden_module_owns_secret_provider_details() {
        let spec = vaultwarden::spec();
        assert_eq!(spec.provider, "vaultwarden.main");
        assert_eq!(spec.container, "gumgum-provider-vaultwarden-main");
        assert_eq!(spec.protocol, "bitwarden-compatible");
        assert!(
            vaultwarden::actions("stripe-api-key", "stripe.secret.example.test")
                .iter()
                .any(|action| action.contains("vaultwarden.main"))
        );
        assert!(
            vaultwarden::connection_examples("stripe-api-key", "stripe.secret.example.test")
                .iter()
                .any(|example| example.contains("bw get item"))
        );
    }

    #[test]
    fn secret_provider_plan_never_contains_secret_values() {
        let plan = object_provider_plan(
            Capability::Secret,
            "stripe-api-key",
            "stripe.secret.example.test",
        );
        let actions = secret::provider_actions(&plan);

        assert_eq!(plan.provider.provider, "onepassword.main");
        assert!(
            actions
                .iter()
                .any(|action| action.contains("no secret value stored"))
        );
        assert!(!actions.iter().any(|action| action.contains("sk_live")));
    }

    #[test]
    fn shell_single_quote_escapes_minio_credentials() {
        assert_eq!(minio::shell_single_quote("plain"), "plain");
        assert_eq!(minio::shell_single_quote("don't"), "don'\\''t");
    }

    #[test]
    fn object_provider_plans_are_actionable() {
        let plan = object_provider_plan(
            Capability::Blob,
            "User Uploads",
            "uploads.blob.example.test",
        );
        assert_eq!(plan.provider.provider, "minio.main");
        assert!(
            plan.actions
                .iter()
                .any(|action| action.contains("ensure bucket user-uploads exists"))
        );
        assert!(
            plan.connection_examples
                .iter()
                .any(|example| example.contains("S3_BUCKET=User Uploads"))
        );
    }
}
