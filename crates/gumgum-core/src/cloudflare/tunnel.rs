use tokio::process::Command as TokioCommand;

use crate::{ErrorCode, GumgumError, Result, Subsystem, process::run_setup_command};

const CLOUDFLARED_CONTAINER: &str = "gumgum-cloudflared";
const CLOUDFLARED_IMAGE: &str = "cloudflare/cloudflared:latest";
const CADDY_NETWORK: &str = "caddy-network";

pub async fn ensure_cloudflared(token: &str) -> Result<Vec<String>> {
    if cloudflared_running_on_caddy_network().await? {
        return Ok(vec![format!(
            "ensure Cloudflare connector container {CLOUDFLARED_CONTAINER}"
        )]);
    }

    let _ = TokioCommand::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(CLOUDFLARED_CONTAINER)
        .output()
        .await;

    run_setup_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(CLOUDFLARED_CONTAINER)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg(CADDY_NETWORK)
            .arg("-e")
            .arg(format!("TUNNEL_TOKEN={token}"))
            .arg(CLOUDFLARED_IMAGE)
            .arg("tunnel")
            .arg("--no-autoupdate")
            .arg("run"),
    )
    .await?;

    Ok(vec![format!(
        "started Cloudflare connector container {CLOUDFLARED_CONTAINER}"
    )])
}

async fn cloudflared_running_on_caddy_network() -> Result<bool> {
    let output = TokioCommand::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg(format!(
            "{{{{.State.Running}}}} {{{{if index .NetworkSettings.Networks \"{CADDY_NETWORK}\"}}}}{CADDY_NETWORK}{{{{end}}}}"
        ))
        .arg(CLOUDFLARED_CONTAINER)
        .output()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to inspect Cloudflare connector container",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == format!("true {CADDY_NETWORK}"))
}
