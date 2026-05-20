use crate::{ErrorCode, GraphStore, GumgumError, Subsystem};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tokio::process::Command as TokioCommand;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployRequest {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: String,
    pub port: u16,
    pub health: String,
}

pub struct ContainerReconciler {
    graph_path: PathBuf,
}

impl ContainerReconciler {
    pub fn new(graph_path: PathBuf) -> Self {
        Self { graph_path }
    }

    pub async fn reconcile(&self, request: &DeployRequest) -> crate::Result<(bool, Vec<String>)> {
        let mut actions = Vec::new();
        let binding_env = self.binding_env(&request.worker)?;
        let inspect = TokioCommand::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{.Config.Image}} {{index .Config.Labels \"caddy\"}} {{index .Config.Labels \"caddy.reverse_proxy\"}}")
            .arg(&request.container)
            .output()
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Setup,
                    ErrorCode::Io,
                    "could not inspect deployment container",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        let current = String::from_utf8_lossy(&inspect.stdout).trim().to_owned();
        let expected_proxy = format!("{{{{upstreams {}}}}}", request.port);
        let expected = format!("{} {} {}", request.image, request.route, expected_proxy);
        let route_label = format!("caddy={}", request.route);
        if inspect.status.success() && current == expected && binding_env.is_empty() {
            actions.push("container already matches desired image".to_owned());
            return Ok((false, actions));
        }
        actions.push(format!("pull {}", request.image));
        run_command_streaming(
            TokioCommand::new("docker").arg("pull").arg(&request.image),
            false,
        )
        .await?;
        let network = if Self::docker_running("gumgum-caddy").await {
            "gumgum-network"
        } else {
            "caddy-network"
        };
        if !binding_env.is_empty() {
            actions.push(format!("project {} binding env var(s)", binding_env.len()));
        }
        actions.push(format!("recreate {}", request.container));
        let _ = run_command_streaming(
            TokioCommand::new("docker")
                .arg("rm")
                .arg("-f")
                .arg(&request.container),
            true,
        )
        .await;
        let mut run = TokioCommand::new("docker");
        run.arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&request.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg(network)
            .arg("--label")
            .arg(route_label)
            .arg("--label")
            .arg(format!(
                "caddy.reverse_proxy={{{{upstreams {}}}}}",
                request.port
            ))
            .arg("--label")
            .arg("caddy.tls=internal");
        for (name, value) in &binding_env {
            run.arg("-e").arg(format!("{name}={value}"));
        }
        run.arg(&request.image);
        run_command_streaming(&mut run, false).await?;
        Self::wait_for_container_health(&request.container, request.port, &request.health).await?;
        Ok((true, actions))
    }

    fn binding_env(&self, worker: &str) -> crate::Result<Vec<(String, String)>> {
        GraphStore::new(self.graph_path.clone()).binding_env(worker)
    }

    async fn docker_running(name: &str) -> bool {
        TokioCommand::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{.State.Running}}")
            .arg(name)
            .output()
            .await
            .map(|output| {
                output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
            })
            .unwrap_or(false)
    }

    async fn wait_for_container_health(
        container: &str,
        port: u16,
        health: &str,
    ) -> crate::Result<()> {
        for _ in 0..20 {
            let output = TokioCommand::new("docker")
                .arg("inspect")
                .arg("-f")
                .arg("{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}")
                .arg(container)
                .output()
                .await
                .map_err(|source| {
                    GumgumError::structured(
                        Subsystem::Setup,
                        ErrorCode::Io,
                        "could not inspect deployment IP",
                    )
                    .likely_cause(source.to_string())
                    .build()
                })?;
            let ip = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !ip.is_empty() {
                let url = format!("http://{ip}:{port}{health}");
                if reqwest::get(&url)
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false)
                {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "deployment container did not become healthy",
        )
        .build())
    }
}

async fn run_command_streaming(cmd: &mut TokioCommand, quiet: bool) -> crate::Result<()> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "failed to run command")
            .likely_cause(source.to_string())
            .build()
    })?;
    if !quiet {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if output.status.success() || quiet {
        Ok(())
    } else {
        Err(
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "command failed")
                .likely_cause(format!("process exited with {}", output.status))
                .build(),
        )
    }
}
