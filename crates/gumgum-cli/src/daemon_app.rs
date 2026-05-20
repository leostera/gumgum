use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    routing::{get, post},
};
use gumgum_api::{
    AffectedReport, BindingReport, BindingRequest, DeployApplyReport, DeployRequest,
    DeploymentRevisionsReport, GraphNode, GraphReport, LogsReport, ObjectReport, ObjectRequest,
    RollbackReport, RollbackRequest,
};
use gumgum_core::{
    ConfigStore, ContainerReconciler, DeployRequest as CoreDeployRequest, DesiredDeploy, ErrorCode,
    GlobalObject, GraphStore, GumgumError, LocalPlatform, Subsystem, WorkerBinding,
    affected_subgraph, connection_examples, not_configured_status, object_dns, provider_for_object,
    render_mermaid_graph,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
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
        LocalPlatform::ensure(false).await?;
        let graph_path = ConfigStore::from_home_env()?.root().join("graph.sqlite");
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
            .route("/v0/revisions/{worker}", get(daemon_revisions))
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
    let graph = render_mermaid_graph(&nodes, &edges);
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
struct RevisionsQuery {
    tail: Option<u32>,
    limit: Option<u32>,
}

async fn daemon_revisions(
    State(state): State<DaemonState>,
    AxumPath(worker): AxumPath<String>,
    Query(query): Query<RevisionsQuery>,
) -> Json<DeploymentRevisionsReport> {
    let path = (*state.graph_path).clone();
    let worker_for_task = worker.clone();
    let limit = query.limit.or(query.tail).unwrap_or(10);
    let revisions = tokio::task::spawn_blocking(move || {
        GraphStore::new(path).deployment_revisions(&worker_for_task, limit)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default();
    Json(DeploymentRevisionsReport {
        ok: true,
        worker,
        message: format!("{} deployment revision(s)", revisions.len()),
        revisions,
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
    let revision_id = request.revision_id;
    let rollback_revision =
        tokio::task::spawn_blocking(move || store.rollback_revision(&worker, revision_id))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
    if let Some(revision) = rollback_revision {
        let deploy = revision.deploy.clone();
        let image = deploy.image.clone();
        let actions = if request.preview {
            rollback_preview_actions(&image)
        } else {
            let store = GraphStore::new((*state.graph_path).clone());
            let deploy_for_db = deploy.clone();
            let _ =
                tokio::task::spawn_blocking(move || store.materialize_deploy(&deploy_for_db)).await;
            let deploy_request = deploy_request_from_desired(deploy);
            let (_, mut actions) = ContainerReconciler::new((*state.graph_path).clone())
                .reconcile(&core_deploy_request(&deploy_request))
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
            actions
        };
        Json(rollback_report_from_revision(
            request.worker,
            request.preview,
            revision,
            actions,
        ))
    } else {
        Json(rollback_unavailable_report(
            request.worker,
            request.revision_id,
        ))
    }
}

fn rollback_preview_actions(image: &str) -> Vec<String> {
    vec![
        format!("would rollback to {image}"),
        "preview only; no containers changed".to_owned(),
    ]
}

fn rollback_report_from_revision(
    worker: String,
    preview: bool,
    revision: gumgum_core::DeploymentRevision,
    actions: Vec<String>,
) -> RollbackReport {
    RollbackReport {
        ok: true,
        worker,
        image: Some(revision.deploy.image),
        revision_id: Some(revision.id),
        container: Some(revision.deploy.container),
        route: Some(revision.deploy.route),
        port: Some(revision.deploy.port),
        health: Some(revision.deploy.health),
        actions,
        message: if preview {
            "rollback preview"
        } else {
            "rollback applied"
        }
        .to_owned(),
    }
}

fn rollback_unavailable_report(worker: String, revision_id: Option<i64>) -> RollbackReport {
    RollbackReport {
        ok: false,
        worker,
        image: None,
        revision_id,
        container: None,
        route: None,
        port: None,
        health: None,
        actions: vec![match revision_id {
            Some(id) => format!("revision {id} not found"),
            None => "no previous image recorded".to_owned(),
        }],
        message: match revision_id {
            Some(id) => format!("deployment revision {id} not found"),
            None => "no previous deployment image recorded".to_owned(),
        },
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
    let (changed, actions) = ContainerReconciler::new(reconcile_path)
        .reconcile(&core_deploy_request(&request))
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

fn core_deploy_request(value: &DeployRequest) -> CoreDeployRequest {
    CoreDeployRequest {
        worker: value.worker.clone(),
        image: value.image.clone(),
        container: value.container.clone(),
        route: value.route.clone(),
        port: value.port,
        health: value.health.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_graph_path(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("gumgum-daemon-{label}-{nanos}.sqlite"))
    }

    fn seed_revisions(path: &PathBuf) -> Vec<gumgum_core::DeploymentRevision> {
        let store = GraphStore::new(path.clone());
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: "api.example.test".to_owned(),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        let mut second = first.clone();
        second.image = "registry/api:2".to_owned();
        store.materialize_deploy(&second).unwrap();
        let mut third = second.clone();
        third.route = "api-v3.example.test".to_owned();
        store.materialize_deploy(&third).unwrap();
        store.deployment_revisions("api", 10).unwrap()
    }

    fn revision(id: i64) -> gumgum_core::DeploymentRevision {
        gumgum_core::DeploymentRevision {
            id,
            created_at: "2026-05-20 12:00:00".to_owned(),
            deploy: DesiredDeploy {
                worker: "api".to_owned(),
                image: "registry/api:1".to_owned(),
                container: "gumgum-api".to_owned(),
                route: "api.example.test".to_owned(),
                port: 3000,
                health: "/healthz".to_owned(),
            },
        }
    }

    #[test]
    fn rollback_report_helper_preserves_apply_target_metadata() {
        let report = rollback_report_from_revision(
            "api".to_owned(),
            false,
            revision(42),
            vec!["rollback to registry/api:1".to_owned()],
        );

        assert!(report.ok);
        assert_eq!(report.message, "rollback applied");
        assert_eq!(report.revision_id, Some(42));
        assert_eq!(report.image.as_deref(), Some("registry/api:1"));
        assert_eq!(report.container.as_deref(), Some("gumgum-api"));
        assert_eq!(report.route.as_deref(), Some("api.example.test"));
        assert_eq!(report.port, Some(3000));
        assert_eq!(report.health.as_deref(), Some("/healthz"));
        assert_eq!(report.actions, vec!["rollback to registry/api:1"]);
    }

    #[test]
    fn rollback_unavailable_helper_distinguishes_missing_revision() {
        let report = rollback_unavailable_report("api".to_owned(), Some(99));

        assert!(!report.ok);
        assert_eq!(report.revision_id, Some(99));
        assert_eq!(report.actions, vec!["revision 99 not found"]);
        assert_eq!(report.message, "deployment revision 99 not found");
    }

    #[tokio::test]
    async fn rollback_preview_can_select_specific_revision_without_reconcile() {
        let path = temp_graph_path("specific-rollback-preview");
        let revisions = seed_revisions(&path);
        let selected = revisions[1].clone();
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(report) = daemon_rollback(
            State(state),
            Json(RollbackRequest {
                worker: "api".to_owned(),
                preview: true,
                revision_id: Some(selected.id),
            }),
        )
        .await;

        assert!(report.ok);
        assert_eq!(report.revision_id, Some(selected.id));
        assert_eq!(report.image, Some(selected.deploy.image));
        assert_eq!(report.route, Some(selected.deploy.route));
        assert!(
            report
                .actions
                .iter()
                .any(|action| action == "preview only; no containers changed")
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn rollback_preview_defaults_to_latest_revision_id() {
        let path = temp_graph_path("latest-rollback-preview");
        let revisions = seed_revisions(&path);
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(report) = daemon_rollback(
            State(state),
            Json(RollbackRequest {
                worker: "api".to_owned(),
                preview: true,
                revision_id: None,
            }),
        )
        .await;

        assert!(report.ok);
        assert_eq!(report.revision_id, Some(revisions[0].id));
        assert_eq!(report.image, Some(revisions[0].deploy.image.clone()));
        let _ = std::fs::remove_file(path);
    }
}
