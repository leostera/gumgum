use crate::{ErrorCode, GumgumError, Subsystem};
use tokio::process::Command as TokioCommand;

use super::types::ProviderSpec;

pub(crate) async fn ensure_network() -> crate::Result<()> {
    run_provider_command(
        TokioCommand::new("sh").arg("-c").arg(
            "docker network inspect gumgum-network >/dev/null 2>&1 || docker network create gumgum-network >/dev/null",
        ),
        "could not ensure GumGum provider network",
    )
    .await
}

pub(crate) async fn inspect(container: &str) -> bool {
    TokioCommand::new("docker")
        .arg("inspect")
        .arg(container)
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub(crate) async fn running(container: &str) -> bool {
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

pub(crate) async fn run_provider_command(
    cmd: &mut TokioCommand,
    message: &str,
) -> crate::Result<()> {
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

pub(crate) fn created_provider_actions(provider: &ProviderSpec) -> Vec<String> {
    vec![format!(
        "created {} provider container {}",
        provider.provider, provider.container
    )]
}

pub(crate) async fn start_existing(
    provider: &ProviderSpec,
    message: &str,
) -> crate::Result<Vec<String>> {
    run_provider_command(
        TokioCommand::new("docker")
            .arg("start")
            .arg(&provider.container),
        message,
    )
    .await?;
    Ok(vec![format!(
        "started existing {} provider",
        provider.provider
    )])
}
