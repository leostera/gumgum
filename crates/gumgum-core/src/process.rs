use crate::{ErrorCode, ErrorKind, GumgumError, Subsystem};
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
        .map_err(|source| setup_error(ErrorKind::SetupCommandSpawnFailed, source))?;
    if status.success() {
        Ok(())
    } else {
        Err(GumgumError::structured_kind(
            Subsystem::Setup,
            ErrorCode::Io,
            ErrorKind::SetupCommandFailed,
        )
        .likely_cause(exit_status_cause(status))
        .next_command("gumgum setup <host> --domain <domain> --dry-run")
        .build())
    }
}

pub async fn run_setup_command(cmd: &mut TokioCommand) -> crate::Result<()> {
    let output = cmd
        .output()
        .await
        .map_err(|source| setup_error(ErrorKind::SetupCommandSpawnFailed, source))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(GumgumError::structured_kind(
        Subsystem::Setup,
        ErrorCode::Io,
        ErrorKind::SetupCommandFailed,
    )
    .likely_cause(if stderr.is_empty() {
        exit_status_cause(output.status)
    } else {
        stderr
    })
    .next_command("gumgum setup <host> --domain <domain> --dry-run")
    .build())
}

fn exit_status_cause(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit_status={code}"))
        .unwrap_or_else(|| "exit_status=unknown".to_owned())
}

fn setup_error(kind: ErrorKind, source: impl std::fmt::Display) -> GumgumError {
    GumgumError::structured_kind(Subsystem::Setup, ErrorCode::Io, kind)
        .likely_cause(source.to_string())
        .build()
}
