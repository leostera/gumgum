use crate::{ErrorCode, GumgumError, Subsystem};
use std::process::Stdio;
use tokio::process::Command as TokioCommand;

pub async fn run_setup_command_streaming(cmd: &mut TokioCommand, quiet: bool) -> crate::Result<()> {
    if quiet {
        return run_setup_command(cmd).await;
    }
    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to run setup command",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "setup command failed")
                .likely_cause(format!("exit status {status}"))
                .next_command("gumgum setup <host> --root-domain <domain> --dry-run")
                .build(),
        )
    }
}

pub async fn run_setup_command(cmd: &mut TokioCommand) -> crate::Result<()> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "failed to run setup command",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "setup command failed")
            .likely_cause(if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            })
            .next_command("gumgum setup <host> --root-domain <domain> --dry-run")
            .build(),
    )
}
