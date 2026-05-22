use crate::{ErrorCode, GraphStore, GumgumError, Subsystem};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tokio::process::Command as TokioCommand;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployRequest {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: Option<String>,
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
        let binding_env = self.binding_env(logical_worker(&request.worker))?;
        let binding_env_fingerprint = binding_env_fingerprint(&binding_env);
        let inspect = TokioCommand::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{.Config.Image}} {{index .Config.Labels \"caddy\"}} {{index .Config.Labels \"caddy.reverse_proxy\"}} {{index .Config.Labels \"gumgum.binding_env\"}}")
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
        let expected_proxy = request
            .route
            .as_ref()
            .map(|_| format!("{{{{upstreams {}}}}}", request.port))
            .unwrap_or_default();
        let expected_route = request.route.clone().unwrap_or_default();
        let expected = format!(
            "{} {} {} {}",
            request.image, expected_route, expected_proxy, binding_env_fingerprint
        );
        if inspect.status.success() && current == expected {
            actions.push("container already matches desired image, route, and bindings".to_owned());
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
            .arg(network);
        if let Some(route) = &request.route {
            run.arg("--label")
                .arg(format!("caddy={route}"))
                .arg("--label")
                .arg(format!(
                    "caddy.reverse_proxy={{{{upstreams {}}}}}",
                    request.port
                ))
                .arg("--label")
                .arg("caddy.tls=internal");
        }
        run.arg("--label")
            .arg("gumgum.managed=deployment")
            .arg("--label")
            .arg(format!("gumgum.worker={}", request.worker))
            .arg("--label")
            .arg(format!("gumgum.binding_env={binding_env_fingerprint}"));
        for (name, value) in &binding_env {
            run.arg("-e").arg(format!("{name}={value}"));
        }
        run.arg(&request.image);
        run_command_streaming(&mut run, false).await?;
        if network != "gumgum-network" && Self::docker_network_exists("gumgum-network").await {
            actions.push(format!("connect {} to gumgum-network", request.container));
            run_command_streaming(
                TokioCommand::new("docker")
                    .arg("network")
                    .arg("connect")
                    .arg("gumgum-network")
                    .arg(&request.container),
                false,
            )
            .await?;
        }
        Self::wait_for_container_health(&request.container, request.port, &request.health).await?;
        actions.extend(Self::remove_stale_worker_containers(request).await?);
        actions.extend(Self::remove_stale_route_containers(request).await?);
        Ok((true, actions))
    }

    async fn remove_stale_worker_containers(request: &DeployRequest) -> crate::Result<Vec<String>> {
        let output = TokioCommand::new("docker")
            .arg("ps")
            .arg("-a")
            .arg("--filter")
            .arg(format!("label=gumgum.worker={}", request.worker))
            .arg("--format")
            .arg("{{.Names}}")
            .output()
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Setup,
                    ErrorCode::Io,
                    "could not list deployment containers",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let mut actions = Vec::new();
        for container in String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|container| !container.is_empty() && *container != request.container)
        {
            actions.push(format!("remove stale deployment container {container}"));
            run_command_streaming(
                TokioCommand::new("docker")
                    .arg("rm")
                    .arg("-f")
                    .arg(container),
                true,
            )
            .await?;
        }
        Ok(actions)
    }

    async fn remove_stale_route_containers(request: &DeployRequest) -> crate::Result<Vec<String>> {
        let Some(route) = request.route.as_deref() else {
            return Ok(Vec::new());
        };
        Self::remove_stale_containers(
            request,
            vec![
                "label=gumgum.managed=deployment".to_owned(),
                format!("label=caddy={route}"),
            ],
            "remove stale deployment container for route",
        )
        .await
    }

    async fn remove_stale_containers(
        request: &DeployRequest,
        filters: Vec<String>,
        action_prefix: &str,
    ) -> crate::Result<Vec<String>> {
        let mut command = TokioCommand::new("docker");
        command.arg("ps").arg("-a");
        for filter in filters {
            command.arg("--filter").arg(filter);
        }
        let output = command
            .arg("--format")
            .arg("{{.Names}}")
            .output()
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Setup,
                    ErrorCode::Io,
                    "could not list deployment containers",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let mut actions = Vec::new();
        for container in String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|container| !container.is_empty() && *container != request.container)
        {
            actions.push(format!("{action_prefix} {container}"));
            run_command_streaming(
                TokioCommand::new("docker")
                    .arg("rm")
                    .arg("-f")
                    .arg(container),
                true,
            )
            .await?;
        }
        Ok(actions)
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

    async fn docker_network_exists(name: &str) -> bool {
        TokioCommand::new("docker")
            .arg("network")
            .arg("inspect")
            .arg(name)
            .output()
            .await
            .map(|output| output.status.success())
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
                .arg("{{range.NetworkSettings.Networks}}{{println .IPAddress}}{{end}}")
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
            let ips = String::from_utf8_lossy(&output.stdout);
            for ip in ips.lines().map(str::trim).filter(|ip| !ip.is_empty()) {
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

fn logical_worker(worker: &str) -> &str {
    worker.split_once('@').map_or(worker, |(name, _)| name)
}

fn binding_env_fingerprint(env: &[(String, String)]) -> String {
    let mut entries = env.to_vec();
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut hash = 0xcbf29ce484222325u64;
    for (name, value) in entries {
        for byte in name.bytes().chain([b'=']).chain(value.bytes()).chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("env-{hash:016x}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_worker_strips_deployment_env_suffix() {
        assert_eq!(logical_worker("api@preview"), "api");
        assert_eq!(logical_worker("api@release"), "api");
        assert_eq!(logical_worker("worker"), "worker");
    }

    #[test]
    fn binding_env_fingerprint_is_stable_and_order_insensitive() {
        let left = vec![
            ("DATABASE_URL".to_owned(), "postgres://db".to_owned()),
            ("USER_COUNTERS".to_owned(), "redis://kv".to_owned()),
        ];
        let right = vec![
            ("USER_COUNTERS".to_owned(), "redis://kv".to_owned()),
            ("DATABASE_URL".to_owned(), "postgres://db".to_owned()),
        ];

        assert_eq!(
            binding_env_fingerprint(&left),
            binding_env_fingerprint(&right)
        );
    }

    #[test]
    fn binding_env_fingerprint_changes_when_secret_projection_changes() {
        let old = vec![("DATABASE_URL".to_owned(), "postgres://old".to_owned())];
        let new = vec![("DATABASE_URL".to_owned(), "postgres://new".to_owned())];

        assert_ne!(binding_env_fingerprint(&old), binding_env_fingerprint(&new));
    }
}
