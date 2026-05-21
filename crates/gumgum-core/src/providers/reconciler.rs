use crate::{Capability, ErrorCode, GumgumError, Subsystem, sanitize_name};
use tokio::process::Command as TokioCommand;

pub struct ProviderReconciler;

impl ProviderReconciler {
    pub async fn ensure(plan: &super::types::ObjectProviderPlan) -> crate::Result<Vec<String>> {
        Self::ensure_with_credentials(plan, None).await
    }

    pub async fn ensure_with_credentials(
        plan: &super::types::ObjectProviderPlan,
        credentials: Option<super::types::ProviderCredentials>,
    ) -> crate::Result<Vec<String>> {
        match plan.capability {
            Capability::Kv => ensure_redis(&plan.provider).await,
            Capability::Blob => {
                ensure_minio(
                    plan,
                    credentials.unwrap_or_else(super::types::ProviderCredentials::minio_local_dev),
                )
                .await
            }
            Capability::Secret => Ok(secret_provider_actions(plan)),
            _ => Ok(plan.actions.clone()),
        }
    }

    pub async fn boot_defaults(
        credentials: &[(String, super::types::ProviderCredentials)],
    ) -> crate::Result<Vec<String>> {
        let mut actions = Vec::new();
        actions.extend(
            ensure_postgres(
                &super::specs::provider_spec(Capability::Db),
                provider_credentials(credentials, "postgres.main")?,
            )
            .await?,
        );
        actions.extend(
            ensure_redis_with_credentials(
                &super::specs::provider_spec(Capability::Kv),
                provider_credentials(credentials, "redis.main")?,
            )
            .await?,
        );
        actions.extend(
            ensure_minio_provider(
                &super::specs::provider_spec(Capability::Blob),
                provider_credentials(credentials, "minio.main")?,
            )
            .await?,
        );
        Ok(actions)
    }

    pub async fn ensure_configured_provider(
        config: &super::types::ProviderConfig,
    ) -> crate::Result<Vec<String>> {
        match (config.capability, config.kind.as_str()) {
            (Capability::Secret, "vaultwarden") | (Capability::Secret, "bitwarden") => {
                ensure_vaultwarden(&super::specs::vaultwarden_spec()).await
            }
            _ => Ok(vec![format!(
                "configured {} provider {}",
                config.capability, config.provider
            )]),
        }
    }

    pub async fn statuses() -> Vec<super::types::ProviderStatus> {
        let mut statuses = Vec::new();
        for capability in [
            Capability::Db,
            Capability::Kv,
            Capability::Blob,
            Capability::Queue,
            Capability::Secret,
            Capability::Observability,
        ] {
            let spec = super::specs::provider_spec(capability);
            let running = docker_running(&spec.container).await;
            statuses.push(super::types::ProviderStatus {
                capability,
                provider: spec.provider,
                container: spec.container,
                image: spec.image,
                port: spec.port,
                running,
            });
        }
        let vaultwarden = super::specs::vaultwarden_spec();
        let running = docker_running(&vaultwarden.container).await;
        statuses.push(super::types::ProviderStatus {
            capability: Capability::Secret,
            provider: vaultwarden.provider,
            container: vaultwarden.container,
            image: vaultwarden.image,
            port: vaultwarden.port,
            running,
        });
        statuses
    }
}

pub(crate) fn provider_credentials(
    credentials: &[(String, super::types::ProviderCredentials)],
    provider: &str,
) -> crate::Result<super::types::ProviderCredentials> {
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
    provider: &super::types::ProviderSpec,
    credentials: super::types::ProviderCredentials,
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

async fn ensure_vaultwarden(provider: &super::types::ProviderSpec) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if docker_inspect(&provider.container).await {
        run_provider_command(
            TokioCommand::new("docker")
                .arg("start")
                .arg(&provider.container),
            "could not start vaultwarden provider",
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
        "could not create vaultwarden provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

pub(crate) fn secret_provider_actions(plan: &super::types::ObjectProviderPlan) -> Vec<String> {
    vec![
        "secret provider is external; no secret value stored in GumGum graph".to_owned(),
        format!(
            "mapped secret {} through {}",
            sanitize_name(&plan.name),
            plan.provider.provider
        ),
        "configure 1Password Connect credentials before runtime resolution".to_owned(),
    ]
}

async fn ensure_redis(provider: &super::types::ProviderSpec) -> crate::Result<Vec<String>> {
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
    provider: &super::types::ProviderSpec,
    credentials: super::types::ProviderCredentials,
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
    plan: &super::types::ObjectProviderPlan,
    credentials: super::types::ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_minio_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    ensure_minio_bucket(&bucket, &credentials).await?;
    actions.push(format!("ensured bucket {bucket} on {}", provider.provider));
    Ok(actions)
}

async fn ensure_minio_provider(
    provider: &super::types::ProviderSpec,
    credentials: super::types::ProviderCredentials,
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

async fn ensure_minio_bucket(
    bucket: &str,
    credentials: &super::types::ProviderCredentials,
) -> crate::Result<()> {
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

pub(crate) fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

pub(crate) fn created_provider_actions(provider: &super::types::ProviderSpec) -> Vec<String> {
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

#[allow(dead_code)]
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
        Capability::Secret => vec![
            format!("op item get {name} --vault gumgum"),
            format!("onepassword://gumgum/{name}"),
        ],
        Capability::Observability => vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")],
        Capability::Manual => Vec::new(),
    }
}

#[allow(dead_code)]
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
        Capability::Secret => vec![
            "configure onepassword.main provider credentials".to_owned(),
            format!("map secret {safe_name} from 1Password item"),
            "do not materialize secret values in the graph".to_owned(),
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
