use crate::{ensure_local_platform, gumgum_root};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    routing::{get, post},
};
use gumgum_api::{
    AffectedReport, BindingReport, BindingRequest, DeployApplyReport, DeployRequest, GraphEdge,
    GraphNode, GraphReport, LogsReport, ObjectReport, ObjectRequest, RollbackReport,
    RollbackRequest, not_configured_status,
};
use gumgum_core::{
    DesiredDeploy, ErrorCode, GlobalObject, GraphStore, GumgumError, Subsystem, WorkerBinding,
    connection_examples, object_dns, provider_for_object,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::process::Command as TokioCommand;

#[derive(Clone)]
pub(crate) struct DaemonState {
    pub(crate) graph_path: Arc<PathBuf>,
}

pub(crate) struct DaemonApp;

impl DaemonApp {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn run(self) -> gumgum_core::Result<()> {
        ensure_local_platform(false).await?;
        let graph_path = gumgum_root()?.join("graph.sqlite");
        GraphStore::new(graph_path.clone()).init()?;
        let app = self.router(DaemonState {
            graph_path: Arc::new(graph_path),
        });
        let addr = SocketAddr::from(([0, 0, 0, 0], 7777));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Api,
                    ErrorCode::Io,
                    "could not bind gumgum daemon",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        tracing::info!(%addr, "gumgum daemon listening");
        axum::serve(listener, app).await.map_err(|source| {
            GumgumError::structured(Subsystem::Api, ErrorCode::Io, "gumgum daemon failed")
                .likely_cause(source.to_string())
                .build()
        })
    }

    fn router(&self, state: DaemonState) -> Router {
        Router::new()
            .route("/healthz", get(healthz))
            .route("/v0/status", get(status))
            .route("/v0/deploy", post(daemon_deploy))
            .route("/v0/rollback", post(daemon_rollback))
            .route("/v0/objects", post(daemon_create_object))
            .route("/v0/bindings", post(daemon_create_binding))
            .route("/v0/graph", get(daemon_graph))
            .route("/v0/graph/affected", get(daemon_graph_affected))
            .route("/v0/logs/{container}", get(daemon_logs))
            .with_state(state)
    }
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "gumgumd" }))
}

async fn status() -> Json<gumgum_core::StatusReport> {
    Json(not_configured_status())
}

async fn daemon_graph(State(state): State<DaemonState>) -> Json<GraphReport> {
    let store = GraphStore::new((*state.graph_path).clone());
    let (nodes, edges) = tokio::task::spawn_blocking(move || store.load_graph())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| {
            (
                vec![GraphNode {
                    id: "gumgumd".to_owned(),
                    kind: "daemon".to_owned(),
                    label: "gumgumd".to_owned(),
                }],
                Vec::new(),
            )
        });
    let graph = crate::graph_presenter::GraphPresenter::new().mermaid(&nodes, &edges);
    Json(GraphReport {
        ok: true,
        format: "mermaid".to_owned(),
        graph,
        nodes,
        edges,
    })
}

#[derive(Debug, serde::Deserialize)]
struct AffectedQuery {
    target: String,
}

async fn daemon_graph_affected(
    State(state): State<DaemonState>,
    Query(query): Query<AffectedQuery>,
) -> Json<AffectedReport> {
    let path = (*state.graph_path).clone();
    let target = query.target;
    let target_for_task = target.clone();
    let (nodes, edges) = tokio::task::spawn_blocking(move || {
        let (nodes, edges) = GraphStore::new(path).load_graph()?;
        Ok::<_, GumgumError>(affected_subgraph(&nodes, &edges, &target_for_task))
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_else(|| (Vec::new(), Vec::new()));
    let message = format!("{} affected node(s)", nodes.len());
    Json(AffectedReport {
        ok: true,
        target,
        nodes,
        edges,
        message,
    })
}

fn affected_subgraph(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
    target: &str,
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut seen = std::collections::BTreeSet::new();
    let mut edge_seen = std::collections::BTreeSet::new();
    seen.insert(target.to_owned());

    let mut add_edge = |edge: &GraphEdge, seen: &mut std::collections::BTreeSet<String>| {
        edge_seen.insert((edge.from.clone(), edge.to.clone(), edge.kind.clone()));
        seen.insert(edge.from.clone());
        seen.insert(edge.to.clone());
    };

    for edge in edges {
        if edge.to == target || edge.from == target {
            add_edge(edge, &mut seen);
        }
    }

    let bindings = seen
        .iter()
        .filter(|id| id.starts_with("binding/"))
        .cloned()
        .collect::<Vec<_>>();
    for binding in bindings {
        for edge in edges {
            if edge.to == binding || edge.from == binding {
                add_edge(edge, &mut seen);
            }
        }
    }

    let workers = seen
        .iter()
        .filter(|id| id.starts_with("worker/"))
        .cloned()
        .collect::<Vec<_>>();
    for worker in workers {
        for edge in edges {
            if edge.from == worker && matches!(edge.kind.as_str(), "runs" | "owns" | "created_from")
            {
                add_edge(edge, &mut seen);
            }
        }
    }

    let routes = seen
        .iter()
        .filter(|id| id.starts_with("route/"))
        .cloned()
        .collect::<Vec<_>>();
    for route in routes {
        for edge in edges {
            if edge.from == route && edge.kind == "routes_to" {
                add_edge(edge, &mut seen);
            }
        }
    }

    let affected_nodes = nodes
        .iter()
        .filter(|node| seen.contains(&node.id))
        .cloned()
        .collect::<Vec<_>>();
    let affected_edges = edges
        .iter()
        .filter(|edge| edge_seen.contains(&(edge.from.clone(), edge.to.clone(), edge.kind.clone())))
        .cloned()
        .collect::<Vec<_>>();
    (affected_nodes, affected_edges)
}

async fn daemon_create_object(
    State(state): State<DaemonState>,
    Json(request): Json<ObjectRequest>,
) -> Json<ObjectReport> {
    let store = GraphStore::new((*state.graph_path).clone());
    let request_for_db = GlobalObject {
        capability: request.capability,
        name: request.name.clone(),
        namespace: request.namespace.clone(),
        root_domain: request.root_domain.clone(),
    };
    let capability_name = request.capability.to_string();
    let provider = provider_for_object(&capability_name).to_owned();
    let dns = object_dns(&capability_name, &request.name, &request.root_domain);
    let ok = tokio::task::spawn_blocking(move || store.materialize_object(&request_for_db))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    let connection_examples = connection_examples(&capability_name, &request.name, &dns);
    Json(ObjectReport {
        ok,
        kind: capability_name,
        name: request.name,
        dns,
        provider,
        connection_examples,
        message: "global object materialized in graph".to_owned(),
    })
}

async fn daemon_create_binding(
    State(state): State<DaemonState>,
    Json(request): Json<BindingRequest>,
) -> Json<BindingReport> {
    let store = GraphStore::new((*state.graph_path).clone());
    let request_for_db = WorkerBinding {
        capability: request.capability,
        object_name: request.object_name.clone(),
        worker: request.worker.clone(),
        binding: request.binding.clone(),
        access: request.access.clone(),
    };
    let ok = tokio::task::spawn_blocking(move || store.materialize_binding(&request_for_db))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    Json(BindingReport {
        ok,
        object: format!("{}/{}", request.capability, request.object_name),
        worker: request.worker,
        binding: request.binding,
        message: "binding materialized in graph".to_owned(),
    })
}

#[derive(Debug, serde::Deserialize)]
struct LogsQuery {
    tail: Option<u32>,
}

async fn daemon_logs(
    AxumPath(container): AxumPath<String>,
    Query(query): Query<LogsQuery>,
) -> Json<LogsReport> {
    let tail = query.tail.unwrap_or(100);
    let output = TokioCommand::new("docker")
        .arg("logs")
        .arg("--tail")
        .arg(tail.to_string())
        .arg(&container)
        .output()
        .await;
    let logs = match output {
        Ok(output) => {
            let mut logs = String::new();
            logs.push_str(&String::from_utf8_lossy(&output.stdout));
            logs.push_str(&String::from_utf8_lossy(&output.stderr));
            logs
        }
        Err(source) => format!("failed to read logs: {source}\n"),
    };
    Json(LogsReport {
        ok: true,
        container,
        tail,
        logs,
    })
}

async fn daemon_rollback(
    State(state): State<DaemonState>,
    Json(request): Json<RollbackRequest>,
) -> Json<RollbackReport> {
    let store = GraphStore::new((*state.graph_path).clone());
    let worker = request.worker.clone();
    let rollback_request =
        tokio::task::spawn_blocking(move || store.latest_previous_deploy(&worker))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
    if let Some(deploy) = rollback_request {
        let image = deploy.image.clone();
        let store = GraphStore::new((*state.graph_path).clone());
        let deploy_for_db = deploy.clone();
        let _ = tokio::task::spawn_blocking(move || store.materialize_deploy(&deploy_for_db)).await;
        let deploy_request = deploy_request_from_desired(deploy);
        let (_, mut actions) = reconcile_deploy(&(*state.graph_path), &deploy_request)
            .await
            .unwrap_or_else(|error| {
                (
                    false,
                    vec![format!(
                        "rollback reconcile failed: {}",
                        error.to_report().message
                    )],
                )
            });
        actions.insert(0, format!("rollback to {image}"));
        Json(RollbackReport {
            ok: true,
            worker: request.worker,
            image: Some(image),
            actions,
            message: "rollback applied".to_owned(),
        })
    } else {
        Json(RollbackReport {
            ok: false,
            worker: request.worker,
            image: None,
            actions: vec!["no previous image recorded".to_owned()],
            message: "no previous deployment image recorded".to_owned(),
        })
    }
}

async fn daemon_deploy(
    State(state): State<DaemonState>,
    Json(request): Json<DeployRequest>,
) -> Json<DeployApplyReport> {
    let path = (*state.graph_path).clone();
    let reconcile_path = path.clone();
    let store = GraphStore::new(path.clone());
    let request_for_db = desired_from_deploy_request(request.clone());
    let materialized =
        tokio::task::spawn_blocking(move || store.materialize_deploy(&request_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
    let (changed, actions) = reconcile_deploy(&reconcile_path, &request)
        .await
        .unwrap_or_else(|error| {
            (
                false,
                vec![format!("reconcile failed: {}", error.to_report().message)],
            )
        });
    Json(DeployApplyReport {
        ok: materialized,
        worker: request.worker,
        materialized,
        changed,
        actions,
        message: "desired deployment materialized and reconciled".to_owned(),
    })
}

fn desired_from_deploy_request(value: DeployRequest) -> DesiredDeploy {
    DesiredDeploy {
        worker: value.worker,
        image: value.image,
        container: value.container,
        route: value.route,
        port: value.port,
        health: value.health,
    }
}

fn deploy_request_from_desired(value: DesiredDeploy) -> DeployRequest {
    DeployRequest {
        worker: value.worker,
        image: value.image,
        container: value.container,
        route: value.route,
        port: value.port,
        health: value.health,
    }
}

async fn reconcile_deploy(
    path: &PathBuf,
    request: &DeployRequest,
) -> gumgum_core::Result<(bool, Vec<String>)> {
    let mut actions = Vec::new();
    let binding_env = load_binding_env(path, &request.worker)?;
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
    crate::run_command_streaming(
        TokioCommand::new("docker").arg("pull").arg(&request.image),
        false,
    )
    .await?;
    let network = if docker_running("gumgum-caddy").await {
        "gumgum-network"
    } else {
        "caddy-network"
    };
    if !binding_env.is_empty() {
        actions.push(format!("project {} binding env var(s)", binding_env.len()));
    }
    actions.push(format!("recreate {}", request.container));
    let _ = crate::run_command_streaming(
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
    crate::run_command_streaming(&mut run, false).await?;
    wait_for_container_health(&request.container, request.port, &request.health).await?;
    Ok((true, actions))
}

fn load_binding_env(path: &PathBuf, worker: &str) -> gumgum_core::Result<Vec<(String, String)>> {
    GraphStore::new(path.clone()).binding_env(worker)
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
) -> gumgum_core::Result<()> {
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
