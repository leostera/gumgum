use crate::{
    ContainerRunSpec, ContainerSnapshot, CoreAction, CoreActions, DockerEngine, ErrorCode,
    ErrorKind, GraphStore, GumgumError, Subsystem,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployRequest {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: Option<String>,
    pub project: Option<String>,
    pub domain: Option<String>,
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

    pub async fn reconcile(&self, request: &DeployRequest) -> crate::Result<(bool, CoreActions)> {
        let mut actions = Vec::new();
        let docker = DockerEngine::local()?;
        let binding_env = self.binding_env(&request.worker)?;
        let runtime_env = deployment_runtime_env(&binding_env, request);
        let binding_env_fingerprint = binding_env_fingerprint(&runtime_env);
        let expected_environment = deployment_environment(&request.worker);
        let labels = deployment_labels(request, &binding_env_fingerprint, expected_environment);
        let rollout_fingerprint = deployment_rollout_fingerprint(request, &runtime_env);
        let desired_container = rollout_container_name(&request.container, &rollout_fingerprint);

        for container_name in docker
            .list_container_names_by_label(&stale_worker_container_labels(&request.worker))
            .await?
        {
            if docker
                .inspect_container(&container_name)
                .await?
                .is_some_and(|container| deployment_container_matches(&container, request, &labels))
            {
                actions.push(CoreAction::DeploymentContainerMatches {
                    container: container_name.clone(),
                });
                docker.start_container(&container_name).await?;
                let before_cleanup = actions.len();
                actions.extend(
                    Self::remove_stale_worker_containers(&docker, request, &container_name).await?,
                );
                actions.extend(
                    Self::remove_stale_route_containers(&docker, request, &container_name).await?,
                );
                return Ok((actions.len() > before_cleanup, actions));
            }
        }

        actions.push(CoreAction::ImagePulled {
            image: request.image.clone(),
        });
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
                actions.push(CoreAction::NetworkCreated {
                    network: env_network.clone(),
                });
            }
        }
        if !runtime_env.is_empty() {
            actions.push(CoreAction::DeploymentEnvironmentProjected {
                vars: runtime_env.len(),
            });
        }
        actions.push(CoreAction::DeploymentContainerStarted {
            container: desired_container.clone(),
        });
        let _ = docker.remove_container_force(&desired_container).await;
        docker
            .create_and_start_container(ContainerRunSpec {
                name: desired_container.clone(),
                image: request.image.clone(),
                network: network.to_owned(),
                restart_unless_stopped: true,
                labels,
                env: runtime_env,
                binds: Vec::new(),
                ports: Vec::new(),
                command: Vec::new(),
                entrypoint: Vec::new(),
            })
            .await?;
        if network != shared_network && docker.network_exists(shared_network).await.unwrap_or(false)
        {
            actions.push(CoreAction::ContainerConnectedToNetwork {
                container: desired_container.clone(),
                network: shared_network.to_owned(),
            });
            docker
                .connect_container_to_network(&desired_container, shared_network)
                .await?;
        }
        Self::wait_for_container_health(&docker, &desired_container, request.port, &request.health)
            .await?;
        actions.push(CoreAction::DeploymentContainerHealthy {
            container: desired_container.clone(),
        });
        actions.extend(
            Self::remove_stale_worker_containers(&docker, request, &desired_container).await?,
        );
        actions.extend(
            Self::remove_stale_route_containers(&docker, request, &desired_container).await?,
        );
        Ok((true, actions))
    }

    async fn remove_stale_worker_containers(
        docker: &DockerEngine,
        request: &DeployRequest,
        keep_container: &str,
    ) -> crate::Result<CoreActions> {
        Self::remove_stale_containers(
            docker,
            request,
            keep_container,
            stale_worker_container_labels(&request.worker),
            "remove stale deployment container",
        )
        .await
    }

    async fn remove_stale_route_containers(
        docker: &DockerEngine,
        request: &DeployRequest,
        keep_container: &str,
    ) -> crate::Result<CoreActions> {
        let Some(route) = request.route.as_deref() else {
            return Ok(Vec::new());
        };
        let mut labels = vec![
            "gumgum.managed=deployment".to_owned(),
            format!("caddy={route}"),
        ];
        if let Some(environment) = deployment_environment(&request.worker) {
            labels.push(format!("gumgum.environment={environment}"));
        }
        Self::remove_stale_containers(
            docker,
            request,
            keep_container,
            labels,
            "remove stale deployment container for route",
        )
        .await
    }

    async fn remove_stale_containers(
        docker: &DockerEngine,
        _request: &DeployRequest,
        keep_container: &str,
        labels: Vec<String>,
        _action_prefix: &str,
    ) -> crate::Result<CoreActions> {
        let mut actions = Vec::new();
        for container in docker.list_container_names_by_label(&labels).await? {
            if container == keep_container {
                continue;
            }
            actions.push(CoreAction::DeploymentContainerRemoved {
                container: container.clone(),
            });
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
        Err(GumgumError::structured_kind(
            Subsystem::Api,
            ErrorCode::Io,
            ErrorKind::DeploymentContainerHealthCheckFailed,
        )
        .build())
    }
}

fn deployment_labels(
    request: &DeployRequest,
    binding_env_fingerprint: &str,
    expected_environment: Option<&str>,
) -> HashMap<String, String> {
    let mut labels = HashMap::from([
        ("gumgum.managed".to_owned(), "deployment".to_owned()),
        ("gumgum.worker".to_owned(), request.worker.clone()),
        (
            "gumgum.deployment.base_container".to_owned(),
            request.container.clone(),
        ),
        (
            "gumgum.binding_env".to_owned(),
            binding_env_fingerprint.to_owned(),
        ),
        ("prometheus.scrape".to_owned(), "true".to_owned()),
        ("prometheus.port".to_owned(), request.port.to_string()),
        ("prometheus.path".to_owned(), "/_/metrics".to_owned()),
        ("prometheus.label_worker".to_owned(), request.worker.clone()),
    ]);
    if let Some(environment) = expected_environment {
        labels.insert("gumgum.environment".to_owned(), environment.to_owned());
        labels.insert(
            "prometheus.label_environment".to_owned(),
            environment.to_owned(),
        );
    }
    if let Some(project) = &request.project {
        labels.insert("gumgum.project".to_owned(), project.clone());
        labels.insert("prometheus.label_project".to_owned(), project.clone());
    }
    if let Some(domain) = &request.domain {
        labels.insert("gumgum.domain".to_owned(), domain.clone());
        labels.insert("prometheus.label_domain".to_owned(), domain.clone());
    }
    if let Some(route) = &request.route {
        labels.insert("caddy".to_owned(), route.clone());
        labels.insert(
            "caddy.reverse_proxy".to_owned(),
            format!("{{{{upstreams {}}}}}", request.port),
        );
        labels.insert("caddy.tls".to_owned(), "internal".to_owned());
        labels.insert("caddy.tracing".to_owned(), String::new());
        labels.insert(
            "caddy.tracing.span".to_owned(),
            format!("{}-ingress", request.worker),
        );
        labels.insert(
            "caddy.request_header".to_owned(),
            "traceparent 00-{http.vars.trace_id}-{http.vars.span_id}-01".to_owned(),
        );
        labels.insert(
            "caddy.tracing.span_attributes.gumgum_worker".to_owned(),
            request.worker.clone(),
        );
        if let Some(project) = &request.project {
            labels.insert(
                "caddy.tracing.span_attributes.gumgum_project".to_owned(),
                project.clone(),
            );
        }
        if let Some(domain) = &request.domain {
            labels.insert(
                "caddy.tracing.span_attributes.gumgum_domain".to_owned(),
                domain.clone(),
            );
        }
    }
    labels
}

fn deployment_container_matches(
    container: &ContainerSnapshot,
    request: &DeployRequest,
    expected_labels: &HashMap<String, String>,
) -> bool {
    container.running
        && container.image.as_deref() == Some(request.image.as_str())
        && expected_labels
            .iter()
            .all(|(name, value)| container.labels.get(name) == Some(value))
}

fn deployment_rollout_fingerprint(
    request: &DeployRequest,
    runtime_env: &[(String, String)],
) -> String {
    let mut entries = vec![
        format!("image={}", request.image),
        format!("route={}", request.route.as_deref().unwrap_or_default()),
        format!("port={}", request.port),
        format!("health={}", request.health),
        format!("project={}", request.project.as_deref().unwrap_or_default()),
        format!("domain={}", request.domain.as_deref().unwrap_or_default()),
    ];
    entries.extend(
        runtime_env
            .iter()
            .map(|(name, value)| format!("env:{name}={value}")),
    );
    entries.sort();
    fnv_hash(entries.join("\0").as_bytes())
}

fn rollout_container_name(base: &str, fingerprint: &str) -> String {
    let max_base_len = 63usize.saturating_sub(fingerprint.len() + 1);
    let trimmed = base.chars().take(max_base_len).collect::<String>();
    format!("{trimmed}-{fingerprint}")
}

fn fnv_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn stale_worker_container_labels(worker: &str) -> Vec<String> {
    vec![
        "gumgum.managed=deployment".to_owned(),
        format!("gumgum.worker={worker}"),
    ]
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

fn deployment_runtime_env(
    binding_env: &[(String, String)],
    request: &DeployRequest,
) -> Vec<(String, String)> {
    let mut env = binding_env.to_vec();
    if !env
        .iter()
        .any(|(name, _)| name == "OTEL_EXPORTER_OTLP_ENDPOINT")
    {
        return env;
    }
    let mut attributes = vec![format!("gumgum.worker={}", request.worker)];
    if let Some(environment) = deployment_environment(&request.worker) {
        attributes.push(format!("deployment.environment={environment}"));
        attributes.push(format!("gumgum.environment={environment}"));
    }
    if let Some(project) = &request.project {
        attributes.push(format!("service.namespace={project}"));
        attributes.push(format!("gumgum.project={project}"));
    }
    if let Some(domain) = &request.domain {
        attributes.push(format!("gumgum.domain={domain}"));
    }
    upsert_env(&mut env, "OTEL_SERVICE_NAME", request.worker.clone());
    upsert_env(
        &mut env,
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "http://gumgum-otel:4317".to_owned(),
    );
    upsert_env(&mut env, "OTEL_TRACES_EXPORTER", "otlp".to_owned());
    upsert_env(&mut env, "OTEL_METRICS_EXPORTER", "none".to_owned());
    upsert_env(&mut env, "OTEL_LOGS_EXPORTER", "none".to_owned());
    upsert_env(&mut env, "OTEL_RESOURCE_ATTRIBUTES", attributes.join(","));
    env
}

fn upsert_env(env: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some((_, existing)) = env.iter_mut().find(|(existing, _)| existing == name) {
        *existing = value;
    } else {
        env.push((name.to_owned(), value));
    }
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
    fn stale_worker_cleanup_is_scoped_to_gumgum_deployments() {
        assert_eq!(
            stale_worker_container_labels("api-preview"),
            vec![
                "gumgum.managed=deployment".to_owned(),
                "gumgum.worker=api-preview".to_owned()
            ]
        );
    }

    #[test]
    fn deployment_environment_extracts_preview_and_prod() {
        assert_eq!(deployment_environment("api-preview"), Some("preview"));
        assert_eq!(deployment_environment("api-prod"), Some("prod"));
        assert_eq!(deployment_environment("api"), None);
    }

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

    #[test]
    fn deployment_runtime_env_adds_trace_resource_attributes() {
        let request = DeployRequest {
            worker: "api-prod".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("kava.fund".to_owned()),
            project: Some("visit-counter".to_owned()),
            domain: Some("kava.fund".to_owned()),
            port: 3000,
            health: "/_/ready".to_owned(),
        };
        let env = deployment_runtime_env(
            &[(
                "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
                "http://observability.platform:4317".to_owned(),
            )],
            &request,
        );

        assert!(env.contains(&("OTEL_SERVICE_NAME".to_owned(), "api-prod".to_owned())));
        assert!(env.contains(&(
            "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
            "http://gumgum-otel:4317".to_owned()
        )));
        assert!(env.contains(&("OTEL_TRACES_EXPORTER".to_owned(), "otlp".to_owned())));
        assert!(env.iter().any(|(name, value)| {
            name == "OTEL_RESOURCE_ATTRIBUTES"
                && value.contains("deployment.environment=prod")
                && value.contains("service.namespace=visit-counter")
                && value.contains("gumgum.domain=kava.fund")
        }));
    }

    #[test]
    fn rollout_container_name_is_stable_for_same_deploy_inputs() {
        let request = DeployRequest {
            worker: "api-prod".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-prod-dev-leostera-visit-counter-api".to_owned(),
            route: Some("kava.fund".to_owned()),
            project: Some("visit-counter".to_owned()),
            domain: Some("kava.fund".to_owned()),
            port: 3000,
            health: "/_/ready".to_owned(),
        };
        let env = vec![("DATABASE_URL".to_owned(), "postgres://db".to_owned())];
        let first = deployment_rollout_fingerprint(&request, &env);
        let second = deployment_rollout_fingerprint(&request, &env);

        assert_eq!(first, second);
        let name = rollout_container_name(&request.container, &first);
        assert!(name.len() <= 63);
        assert!(name.ends_with(&first));
    }

    #[test]
    fn deployment_container_match_uses_labels_not_logical_name() {
        let request = DeployRequest {
            worker: "api-prod".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("kava.fund".to_owned()),
            project: Some("visit-counter".to_owned()),
            domain: Some("kava.fund".to_owned()),
            port: 3000,
            health: "/_/ready".to_owned(),
        };
        let labels = deployment_labels(&request, "env-123", Some("prod"));
        let snapshot = ContainerSnapshot {
            name: "gumgum-api-deadbeef".to_owned(),
            image: Some("registry/api:1".to_owned()),
            labels: labels.clone(),
            running: true,
            ..ContainerSnapshot::default()
        };

        assert!(deployment_container_matches(&snapshot, &request, &labels));
    }
}
