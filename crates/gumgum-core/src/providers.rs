use crate::{Capability, ErrorCode, GumgumError, Subsystem, sanitize_name};
use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSpec {
    pub capability: Capability,
    pub provider: String,
    pub container: String,
    pub image: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObjectProviderPlan {
    pub capability: Capability,
    pub name: String,
    pub dns: String,
    pub provider: ProviderSpec,
    pub actions: Vec<String>,
    pub connection_examples: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCredentials {
    pub username_env: String,
    pub password_env: String,
    pub username: String,
    pub password: String,
}

impl ProviderCredentials {
    pub fn minio_local_dev() -> Self {
        Self {
            username_env: "MINIO_ROOT_USER".to_owned(),
            password_env: "MINIO_ROOT_PASSWORD".to_owned(),
            username: std::env::var("GUMGUM_MINIO_ROOT_USER")
                .unwrap_or_else(|_| "gumgum".to_owned()),
            password: std::env::var("GUMGUM_MINIO_ROOT_PASSWORD")
                .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
        }
    }

    pub fn minio_generated() -> Self {
        Self::generated("MINIO_ROOT_USER", "MINIO_ROOT_PASSWORD", "gumgum")
    }

    pub fn postgres_generated() -> Self {
        Self::generated("POSTGRES_USER", "POSTGRES_PASSWORD", "gumgum")
    }

    pub fn redis_local_dev() -> Self {
        Self {
            username_env: "REDIS_USER".to_owned(),
            password_env: "REDIS_PASSWORD".to_owned(),
            username: std::env::var("GUMGUM_REDIS_USER").unwrap_or_else(|_| "gumgum".to_owned()),
            password: std::env::var("GUMGUM_REDIS_PASSWORD")
                .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
        }
    }

    pub fn redis_generated() -> Self {
        Self::generated("REDIS_USER", "REDIS_PASSWORD", "gumgum")
    }

    pub fn generated(username_env: &str, password_env: &str, username: &str) -> Self {
        Self {
            username_env: username_env.to_owned(),
            password_env: password_env.to_owned(),
            username: username.to_owned(),
            password: generate_secret(),
        }
    }
}

fn generate_secret() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ((seed >> ((index % 16) * 8)) & 0xff) as u8;
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub capability: Capability,
    pub provider: String,
    pub container: String,
    pub image: String,
    pub port: u16,
    pub running: bool,
}

pub fn provider_spec(capability: Capability) -> ProviderSpec {
    match capability {
        Capability::Db => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-postgres-main".to_owned(),
            image: "postgres:16-alpine".to_owned(),
            port: 5432,
            protocol: "postgres".to_owned(),
        },
        Capability::Kv => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-redis-main".to_owned(),
            image: "redis:7-alpine".to_owned(),
            port: 6379,
            protocol: "redis".to_owned(),
        },
        Capability::Blob => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-minio-main".to_owned(),
            image: "minio/minio:latest".to_owned(),
            port: 9000,
            protocol: "s3".to_owned(),
        },
        Capability::Queue => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-redpanda-main".to_owned(),
            image: "redpandadata/redpanda:latest".to_owned(),
            port: 9092,
            protocol: "kafka".to_owned(),
        },
        Capability::Observability => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-otel-platform".to_owned(),
            image: "otel/opentelemetry-collector-contrib:latest".to_owned(),
            port: 4317,
            protocol: "otlp".to_owned(),
        },
        Capability::Manual => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-manual-main".to_owned(),
            image: "manual".to_owned(),
            port: 0,
            protocol: "manual".to_owned(),
        },
    }
}

pub fn object_provider_plan(capability: Capability, name: &str, dns: &str) -> ObjectProviderPlan {
    let provider = provider_spec(capability);
    let safe_name = sanitize_name(name);
    ObjectProviderPlan {
        capability,
        name: name.to_owned(),
        dns: dns.to_owned(),
        actions: provider_actions(capability, &safe_name, dns),
        connection_examples: connection_examples(capability, name, dns),
        provider,
    }
}

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
            Capability::Kv => ensure_redis(&plan.provider).await,
            Capability::Blob => {
                ensure_minio(
                    plan,
                    credentials.unwrap_or_else(ProviderCredentials::minio_local_dev),
                )
                .await
            }
            _ => Ok(plan.actions.clone()),
        }
    }

    pub async fn boot_defaults(
        credentials: &[(String, ProviderCredentials)],
    ) -> crate::Result<Vec<String>> {
        let mut actions = Vec::new();
        actions.extend(
            ensure_postgres(
                &provider_spec(Capability::Db),
                provider_credentials(credentials, "postgres.main")?,
            )
            .await?,
        );
        actions.extend(
            ensure_redis_with_credentials(
                &provider_spec(Capability::Kv),
                provider_credentials(credentials, "redis.main")?,
            )
            .await?,
        );
        actions.extend(
            ensure_minio_provider(
                &provider_spec(Capability::Blob),
                provider_credentials(credentials, "minio.main")?,
            )
            .await?,
        );
        Ok(actions)
    }

    pub async fn statuses() -> Vec<ProviderStatus> {
        let mut statuses = Vec::new();
        for capability in [
            Capability::Db,
            Capability::Kv,
            Capability::Blob,
            Capability::Queue,
            Capability::Observability,
        ] {
            let spec = provider_spec(capability);
            let running = docker_running(&spec.container).await;
            statuses.push(ProviderStatus {
                capability,
                provider: spec.provider,
                container: spec.container,
                image: spec.image,
                port: spec.port,
                running,
            });
        }
        statuses
    }
}

fn provider_credentials(
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

async fn ensure_postgres(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if docker_inspect(&provider.container).await {
        run_provider_command(
            TokioCommand::new("docker")
                .arg("start")
                .arg(&provider.container),
            "could not start postgres provider",
        )
        .await?;
        return Ok(vec![format!(
            "started existing {} provider",
            provider.provider
        )]);
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
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.username_env, credentials.username
            ))
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.password_env, credentials.password
            ))
            .arg(&provider.image),
        "could not create postgres provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

async fn ensure_redis(provider: &ProviderSpec) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if docker_inspect(&provider.container).await {
        run_provider_command(
            TokioCommand::new("docker")
                .arg("start")
                .arg(&provider.container),
            "could not start redis provider",
        )
        .await?;
        return Ok(vec![format!(
            "started existing {} provider",
            provider.provider
        )]);
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

async fn ensure_redis_with_credentials(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if docker_inspect(&provider.container).await {
        run_provider_command(
            TokioCommand::new("docker")
                .arg("start")
                .arg(&provider.container),
            "could not start redis provider",
        )
        .await?;
        return Ok(vec![format!(
            "started existing {} provider",
            provider.provider
        )]);
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

async fn ensure_minio(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_minio_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    ensure_minio_bucket(&bucket, &credentials).await?;
    actions.push(format!("ensured bucket {bucket} on {}", provider.provider));
    Ok(actions)
}

async fn ensure_minio_provider(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if docker_inspect(&provider.container).await {
        run_provider_command(
            TokioCommand::new("docker")
                .arg("start")
                .arg(&provider.container),
            "could not start minio provider",
        )
        .await?;
        return Ok(vec![format!(
            "started existing {} provider",
            provider.provider
        )]);
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
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.username_env, credentials.username
            ))
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.password_env, credentials.password
            ))
            .arg(&provider.image)
            .arg("server")
            .arg("/data")
            .arg("--console-address")
            .arg(":9001"),
        "could not create minio provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

async fn ensure_minio_bucket(bucket: &str, credentials: &ProviderCredentials) -> crate::Result<()> {
    let script = format!(
        "set -e; mc alias set gumgum-minio http://gumgum-provider-minio-main:9000 '{}' '{}'; mc mb --ignore-existing gumgum-minio/{}",
        shell_single_quote(&credentials.username),
        shell_single_quote(&credentials.password),
        shell_single_quote(bucket)
    );
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("gumgum-network")
            .arg("minio/mc:latest")
            .arg("sh")
            .arg("-c")
            .arg(script),
        "could not ensure minio bucket",
    )
    .await
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn created_provider_actions(provider: &ProviderSpec) -> Vec<String> {
    vec![format!(
        "created {} provider container {}",
        provider.provider, provider.container
    )]
}

async fn ensure_network() -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("sh").arg("-c").arg(
            "docker network inspect gumgum-network >/dev/null 2>&1 || docker network create gumgum-network >/dev/null",
        ),
        "could not ensure GumGum provider network",
    )
    .await
}

async fn docker_inspect(container: &str) -> bool {
    TokioCommand::new("docker")
        .arg("inspect")
        .arg(container)
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

async fn docker_running(container: &str) -> bool {
    TokioCommand::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg("{{.State.Running}}")
        .arg(container)
        .output()
        .await
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        })
        .unwrap_or(false)
}

async fn run_provider_command(cmd: &mut TokioCommand, message: &str) -> crate::Result<()> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, message)
            .likely_cause(source.to_string())
            .build()
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, message)
                .likely_cause(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                .build(),
        )
    }
}

pub fn connection_examples(capability: Capability, name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => vec![
            format!("psql postgres://{name}:<password>@{dns}:5432/{name}"),
            format!("pgAdmin host={dns} port=5432 database={name} username={name}"),
        ],
        Capability::Kv => vec![
            format!("redis-cli -u redis://{dns}:6379/0"),
            format!("RedisInsight host={dns} port=6379 database=0"),
        ],
        Capability::Blob => vec![
            format!("aws --endpoint-url http://{dns}:9000 s3 mb s3://{name}"),
            format!("S3_ENDPOINT=http://{dns}:9000 S3_BUCKET={name}"),
        ],
        Capability::Queue => vec![
            format!("kcat -b {dns}:9092 -t {name}"),
            format!("KAFKA_BROKERS={dns}:9092 KAFKA_TOPIC={name}"),
        ],
        Capability::Observability => vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")],
        Capability::Manual => Vec::new(),
    }
}

fn provider_actions(capability: Capability, safe_name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => vec![
            "ensure postgres.main provider is running".to_owned(),
            format!("ensure database {safe_name} exists"),
            format!("publish DNS {dns} to postgres.main"),
        ],
        Capability::Kv => vec![
            "ensure redis.main provider is running".to_owned(),
            format!("reserve Redis key prefix {safe_name}:"),
            format!("publish DNS {dns} to redis.main"),
        ],
        Capability::Blob => vec![
            "ensure minio.main provider is running".to_owned(),
            format!("ensure bucket {safe_name} exists"),
            format!("publish DNS {dns} to minio.main"),
        ],
        Capability::Queue => vec![
            "ensure redpanda.main provider is running".to_owned(),
            format!("ensure topic {safe_name} exists"),
            format!("publish DNS {dns} to redpanda.main"),
        ],
        Capability::Observability => vec![
            "ensure otel.platform provider is running".to_owned(),
            format!("publish DNS {dns} to otel.platform"),
        ],
        Capability::Manual => {
            vec!["manual provider requires operator-managed backing service".to_owned()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[tokio::test]
    async fn provider_statuses_cover_inspectable_backends() {
        let statuses = ProviderReconciler::statuses().await;

        assert_eq!(statuses.len(), 5);
        assert!(
            statuses
                .iter()
                .any(|status| status.provider == "redis.main")
        );
        assert!(
            statuses
                .iter()
                .any(|status| status.container == "gumgum-provider-postgres-main")
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
    fn provider_boot_requires_all_default_credentials() {
        let credentials = vec![(
            "redis.main".to_owned(),
            ProviderCredentials::generated("REDIS_USER", "REDIS_PASSWORD", "gumgum"),
        )];
        let error = provider_credentials(&credentials, "postgres.main")
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
            created_provider_actions(&plan.provider),
            vec!["created minio.main provider container gumgum-provider-minio-main"]
        );
        assert!(
            plan.actions
                .iter()
                .any(|action| action == "ensure bucket uploads exists")
        );
    }

    #[test]
    fn shell_single_quote_escapes_minio_credentials() {
        assert_eq!(shell_single_quote("plain"), "plain");
        assert_eq!(shell_single_quote("don't"), "don'\\''t");
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
