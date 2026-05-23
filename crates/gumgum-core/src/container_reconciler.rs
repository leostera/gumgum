use crate::{ContainerRunSpec, DockerEngine, ErrorCode, GraphStore, GumgumError, Subsystem};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, time::Duration};

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
        let docker = DockerEngine::local()?;
        let binding_env = self.binding_env(&request.worker)?;
        let binding_env_fingerprint = binding_env_fingerprint(&binding_env);
        let expected_proxy = request
            .route
            .as_ref()
            .map(|_| format!("{{{{upstreams {}}}}}", request.port))
            .unwrap_or_default();
        let expected_route = request.route.clone().unwrap_or_default();
        let expected_environment = deployment_environment(&request.worker);
        if docker
            .inspect_container(&request.container)
            .await?
            .is_some_and(|container| {
                container.image.as_deref() == Some(request.image.as_str())
                    && container.labels.get("caddy") == Some(&expected_route)
                    && container.labels.get("caddy.reverse_proxy") == Some(&expected_proxy)
                    && container.labels.get("gumgum.binding_env") == Some(&binding_env_fingerprint)
                    && container
                        .labels
                        .get("gumgum.environment")
                        .map(String::as_str)
                        == expected_environment
            })
        {
            actions.push("container already matches desired image, route, and bindings".to_owned());
            let before_cleanup = actions.len();
            actions.extend(Self::remove_stale_worker_containers(&docker, request).await?);
            actions.extend(Self::remove_stale_route_containers(&docker, request).await?);
            return Ok((actions.len() > before_cleanup, actions));
        }
        actions.push(format!("pull {}", request.image));
        docker.pull_image(&request.image).await?;
        let shared_network = if docker
            .container_running("gumgum-caddy")
            .await
            .unwrap_or(false)
        {
            "gumgum-network"
        } else {
            "caddy-network"
        };
        let env_network = deployment_network_name(&request.worker);
        let network = env_network.as_deref().unwrap_or(shared_network);
        if let Some(env_network) = &env_network {
            if docker.ensure_network(env_network).await? {
                actions.push(format!("create environment network {env_network}"));
            }
        }
        if !binding_env.is_empty() {
            actions.push(format!("project {} binding env var(s)", binding_env.len()));
        }
        actions.push(format!("recreate {}", request.container));
        let _ = docker.remove_container_force(&request.container).await;
        let mut labels = HashMap::from([
            ("gumgum.managed".to_owned(), "deployment".to_owned()),
            ("gumgum.worker".to_owned(), request.worker.clone()),
            (
                "gumgum.binding_env".to_owned(),
                binding_env_fingerprint.clone(),
            ),
        ]);
        if let Some(environment) = expected_environment {
            labels.insert("gumgum.environment".to_owned(), environment.to_owned());
        }
        if let Some(route) = &request.route {
            labels.insert("caddy".to_owned(), route.clone());
            labels.insert(
                "caddy.reverse_proxy".to_owned(),
                format!("{{{{upstreams {}}}}}", request.port),
            );
            labels.insert("caddy.tls".to_owned(), "internal".to_owned());
        }
        docker
            .create_and_start_container(ContainerRunSpec {
                name: request.container.clone(),
                image: request.image.clone(),
                network: network.to_owned(),
                restart_unless_stopped: true,
                labels,
                env: binding_env.clone(),
                binds: Vec::new(),
                ports: Vec::new(),
                command: Vec::new(),
                entrypoint: Vec::new(),
            })
            .await?;
        if network != shared_network && docker.network_exists(shared_network).await.unwrap_or(false)
        {
            actions.push(format!("connect {} to {shared_network}", request.container));
            docker
                .connect_container_to_network(&request.container, shared_network)
                .await?;
        }
        Self::wait_for_container_health(&docker, &request.container, request.port, &request.health)
            .await?;
        actions.extend(Self::remove_stale_worker_containers(&docker, request).await?);
        actions.extend(Self::remove_stale_route_containers(&docker, request).await?);
        Ok((true, actions))
    }

    async fn remove_stale_worker_containers(
        docker: &DockerEngine,
        request: &DeployRequest,
    ) -> crate::Result<Vec<String>> {
        Self::remove_stale_containers(
            docker,
            request,
            vec![format!("gumgum.worker={}", request.worker)],
            "remove stale deployment container",
        )
        .await
    }

    async fn remove_stale_route_containers(
        docker: &DockerEngine,
        request: &DeployRequest,
    ) -> crate::Result<Vec<String>> {
        let Some(route) = request.route.as_deref() else {
            return Ok(Vec::new());
        };
        Self::remove_stale_containers(
            docker,
            request,
            vec![
                "gumgum.managed=deployment".to_owned(),
                format!("caddy={route}"),
            ],
            "remove stale deployment container for route",
        )
        .await
    }

    async fn remove_stale_containers(
        docker: &DockerEngine,
        request: &DeployRequest,
        labels: Vec<String>,
        action_prefix: &str,
    ) -> crate::Result<Vec<String>> {
        let mut actions = Vec::new();
        for container in docker.list_container_names_by_label(&labels).await? {
            if container == request.container {
                continue;
            }
            actions.push(format!("{action_prefix} {container}"));
            docker.remove_container_force(&container).await?;
        }
        Ok(actions)
    }

    fn binding_env(&self, worker: &str) -> crate::Result<Vec<(String, String)>> {
        GraphStore::new(self.graph_path.clone()).binding_env(worker)
    }

    async fn wait_for_container_health(
        docker: &DockerEngine,
        container: &str,
        port: u16,
        health: &str,
    ) -> crate::Result<()> {
        for _ in 0..20 {
            if let Some(container) = docker.inspect_container(container).await? {
                for ip in container.networks.values().filter(|ip| !ip.is_empty()) {
                    let url = format!("http://{ip}:{port}{health}");
                    if reqwest::get(&url)
                        .await
                        .map(|response| response.status().is_success())
                        .unwrap_or(false)
                    {
                        return Ok(());
                    }
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

fn deployment_environment(worker: &str) -> Option<&str> {
    worker
        .strip_suffix("-preview")
        .map(|_| "preview")
        .or_else(|| worker.strip_suffix("-prod").map(|_| "prod"))
        .or_else(|| worker.split_once('@').map(|(_, env)| env))
}

fn deployment_network_name(worker: &str) -> Option<String> {
    deployment_environment(worker).map(|env| format!("gumgum-{env}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_network_name_follows_environment_suffix() {
        assert_eq!(
            deployment_network_name("api-preview").as_deref(),
            Some("gumgum-preview")
        );
        assert_eq!(
            deployment_network_name("api-prod").as_deref(),
            Some("gumgum-prod")
        );
        assert_eq!(
            deployment_network_name("api@preview").as_deref(),
            Some("gumgum-preview")
        );
        assert_eq!(deployment_network_name("api").as_deref(), None);
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
