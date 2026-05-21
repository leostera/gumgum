use crate::{Capability, sanitize_name};
use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ObjectProviderPlan, ProviderCredentials, ProviderSpec};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Blob,
        provider: "minio.main".to_owned(),
        container: "gumgum-provider-minio-main".to_owned(),
        image: "minio/minio:latest".to_owned(),
        port: 9000,
        protocol: "s3".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure minio.main provider is running".to_owned(),
        format!("ensure bucket {safe_name} exists"),
        format!("publish DNS {dns} to minio.main"),
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("aws --endpoint-url http://{dns}:9000 s3 mb s3://{name}"),
        format!("S3_ENDPOINT=http://{dns}:9000 S3_BUCKET={name}"),
    ]
}

pub(crate) async fn ensure(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    ensure_bucket(&bucket, &credentials).await?;
    actions.push(format!("ensured bucket {bucket} on {}", provider.provider));
    actions.push(format!("published DNS {} to minio.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    delete_bucket(&bucket, &credentials).await?;
    actions.push(format!(
        "deleted bucket {bucket} from {}",
        provider.provider
    ));
    actions.push(format!("removed DNS {} from minio.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn ensure_provider(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start minio provider").await;
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

async fn ensure_bucket(bucket: &str, credentials: &ProviderCredentials) -> crate::Result<()> {
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

async fn delete_bucket(bucket: &str, credentials: &ProviderCredentials) -> crate::Result<()> {
    let script = format!(
        "set -e; mc alias set gumgum-minio http://gumgum-provider-minio-main:9000 '{}' '{}'; mc rb --force gumgum-minio/{} || true",
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
        "could not delete minio bucket",
    )
    .await
}

pub(crate) fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}
