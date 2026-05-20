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
        match plan.capability {
            Capability::Kv => ensure_redis(&plan.provider).await,
            _ => Ok(plan.actions.clone()),
        }
    }
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
    Ok(vec![format!(
        "created {} provider container {}",
        provider.provider, provider.container
    )])
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
