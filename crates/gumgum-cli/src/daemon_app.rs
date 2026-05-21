use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    routing::{get, post},
};
use gumgum_api::{
    AffectedReport, BindingDeleteRequest, BindingReport, BindingRequest, DaemonVersionReport,
    DeployApplyReport, DeployRequest, DeploymentDeleteRequest, DeploymentRevisionsReport,
    EnvReport, EnvVar, EventsReport, GraphNode, GraphReport, LogsReport, ObjectDeleteRequest,
    ObjectReport, ObjectRequest, ProviderBootReport, ProviderConfigureReport,
    ProviderConfigureRequest, ProviderCredentialsInitReport, ProviderCredentialsReport,
    ProviderStatusReport, RollbackReport, RollbackRequest,
};
use gumgum_core::{
    ConfigStore, DesiredDeploy, DesiredGraphNode, DesiredProvider, ErrorCode, GlobalObject,
    GraphActionExecutor, GraphActionPlanner, GraphExecutionContext, GraphStore, GumgumError,
    LocalPlatform, ProviderReconciler, Subsystem, WorkerBinding, affected_subgraph,
    not_configured_status, object_dns, object_provider_plan, render_mermaid_graph,
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
            .route("/v0/version", get(daemon_version))
            .route("/v0/status", get(status))
            .route(
                "/v0/deploy",
                post(daemon_deploy).delete(daemon_delete_deploy),
            )
            .route("/v0/rollback", post(daemon_rollback))
            .route("/v0/revisions/{worker}", get(daemon_revisions))
            .route("/v0/events", get(daemon_events))
            .route(
                "/v0/objects",
                post(daemon_create_object).delete(daemon_delete_object),
            )
            .route(
                "/v0/bindings",
                post(daemon_create_binding).delete(daemon_delete_binding),
            )
            .route("/v0/providers", get(daemon_providers))
            .route("/v0/providers/configure", post(daemon_configure_provider))
            .route(
                "/v0/providers/defaults/boot",
                post(daemon_boot_default_providers),
            )
            .route(
                "/v0/providers/minio/credentials/init",
                post(daemon_init_minio_credentials),
            )
            .route("/v0/env/{worker}", get(daemon_env))
            .route("/v0/graph", get(daemon_graph))
            .route("/v0/graph/affected", get(daemon_graph_affected))
            .route("/v0/logs/{container}", get(daemon_logs))
            .with_state(state)
    }
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "gumgumd" }))
}

async fn daemon_version() -> Json<DaemonVersionReport> {
    Json(daemon_version_report())
}

fn daemon_version_report() -> DaemonVersionReport {
    DaemonVersionReport {
        ok: true,
        version: option_env!("GUMGUM_BUILD_VERSION")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_owned(),
        git_sha: option_env!("GUMGUM_BUILD_SHA")
            .unwrap_or("unknown")
            .to_owned(),
        target: option_env!("GUMGUM_BUILD_TARGET")
            .unwrap_or("unknown")
            .to_owned(),
        capabilities: vec![
            "graph".to_owned(),
            "logs".to_owned(),
            "rollback".to_owned(),
            "rollback_revisions".to_owned(),
            "rollback_revision_id".to_owned(),
            "events".to_owned(),
        ],
    }
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

async fn daemon_delete_object(
    State(state): State<DaemonState>,
    Json(request): Json<ObjectDeleteRequest>,
) -> Json<ObjectReport> {
    let graph_path = (*state.graph_path).clone();
    let object = GlobalObject {
        capability: request.capability,
        name: request.name.clone(),
        namespace: request.namespace.clone(),
        root_domain: request.root_domain.clone(),
    };
    let capability_name = request.capability.to_string();
    let dns = object_dns(&capability_name, &request.name, &request.root_domain);
    let provider = request.capability.provider().to_owned();
    let reconciliation_steps = object.delete_reconciliation_steps(graph_path.clone()).await;
    let deleted = if request.preview {
        false
    } else {
        let store = GraphStore::new(graph_path.clone());
        let object_for_db = object.clone();
        tokio::task::spawn_blocking(move || store.delete_object(&object_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    };
    let provider_actions = if request.preview {
        vec!["preview only; no objects changed".to_owned()]
    } else {
        GraphActionExecutor::execute_steps(
            &reconciliation_steps,
            GraphExecutionContext {
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap_or_else(|error| {
            vec![format!(
                "object delete reconcile failed: {}",
                error.to_report().message
            )]
        })
    };
    Json(ObjectReport {
        ok: request.preview || deleted,
        kind: capability_name,
        name: request.name,
        dns,
        provider,
        connection_examples: Vec::new(),
        provider_actions,
        reconciliation_steps,
        message: if request.preview {
            "object delete preview".to_owned()
        } else if deleted {
            "object deleted from graph".to_owned()
        } else {
            "object was not present".to_owned()
        },
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
    let capability = request.capability;
    let capability_name = capability.to_string();
    let dns = object_dns(&capability_name, &request.name, &request.root_domain);
    let provider_plan = object_provider_plan(capability, &request.name, &dns);
    let provider = provider_plan.provider.provider.clone();
    let reconciliation_steps = object_reconciliation_plan(
        (*state.graph_path).clone(),
        capability,
        request.name.clone(),
        provider.clone(),
    )
    .await;
    let provider_credentials = match required_provider_credentials(capability) {
        Ok(credentials) => credentials,
        Err(()) => {
            return Json(missing_provider_credentials_report(
                capability_name,
                request.name,
                dns,
                provider,
                provider_plan.connection_examples,
            ));
        }
    };
    let db_password = if capability == gumgum_core::Capability::Db {
        Some(
            request
                .password
                .clone()
                .unwrap_or_else(gumgum_core::generated_secret_value),
        )
    } else {
        None
    };
    let secret_object_name = format!("{}-password", gumgum_core::sanitize_name(&request.name));
    let object_name_for_secret = request.name.clone();
    let ok = tokio::task::spawn_blocking(move || {
        let ok = store.materialize_object(&request_for_db)?;
        if let Some(password) = db_password {
            store.materialize_object(&GlobalObject {
                capability: gumgum_core::Capability::Secret,
                name: secret_object_name.clone(),
                namespace: request_for_db.namespace.clone(),
                root_domain: request_for_db.root_domain.clone(),
            })?;
            store.materialize_object_secret(
                "db",
                &object_name_for_secret,
                "password",
                "PASSWORD",
                &format!(
                    "onepassword://gumgum/db/{}/password",
                    gumgum_core::sanitize_name(&object_name_for_secret)
                ),
                &password,
            )?;
        }
        Ok::<bool, gumgum_core::GumgumError>(ok)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false);
    let mut execution_steps = reconciliation_steps.clone();
    if execution_steps.is_empty() {
        execution_steps.push(GraphActionPlanner::ensure_object_step(
            capability,
            gumgum_core::ObjectName::new(&request.name)
                .unwrap_or_else(|_| gumgum_core::ObjectName::new("object").unwrap()),
            gumgum_core::ProviderName::new(&provider)
                .unwrap_or_else(|_| gumgum_core::ProviderName::new("provider.local").unwrap()),
        ));
    }
    let provider_actions = GraphActionExecutor::execute_steps(
        &execution_steps,
        GraphExecutionContext {
            object_plan: Some(provider_plan.clone()),
            provider_credentials,
            graph_path: Some((*state.graph_path).clone()),
        },
    )
    .await
    .unwrap_or_else(|error| {
        vec![format!(
            "provider reconcile failed: {}",
            error.to_report().message
        )]
    });
    Json(ObjectReport {
        ok,
        kind: capability_name,
        name: request.name,
        dns,
        provider,
        connection_examples: provider_plan.connection_examples,
        provider_actions,
        reconciliation_steps: execution_steps,
        message: if capability == gumgum_core::Capability::Db {
            "global object materialized with managed password secret and provider reconciled"
        } else {
            "global object materialized and provider reconciled"
        }
        .to_owned(),
    })
}

async fn object_reconciliation_plan(
    graph_path: PathBuf,
    capability: gumgum_core::Capability,
    name: String,
    provider: String,
) -> Vec<gumgum_core::GraphExecutionStep> {
    tokio::task::spawn_blocking(move || {
        let store = GraphStore::new(graph_path);
        let old_graph = store.load_desired_graph()?;
        let mut new_graph = old_graph.clone();
        new_graph.nodes.insert(DesiredGraphNode::Object {
            capability,
            name: gumgum_core::ObjectName::new(&name)?,
            provider: gumgum_core::ProviderName::new(&provider)?,
        });
        Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default()
}

fn required_provider_credentials(
    capability: gumgum_core::Capability,
) -> Result<Option<gumgum_core::ProviderCredentials>, ()> {
    let provider = match capability {
        gumgum_core::Capability::Db => "postgres.main",
        gumgum_core::Capability::Kv => "redis.main",
        gumgum_core::Capability::Blob => "minio.main",
        _ => return Ok(None),
    };
    ConfigStore::from_home_env()
        .and_then(|store| store.load_provider_credentials(provider))
        .ok()
        .flatten()
        .map(Some)
        .ok_or(())
}

fn missing_provider_credentials_report(
    kind: String,
    name: String,
    dns: String,
    provider: String,
    connection_examples: Vec<String>,
) -> ObjectReport {
    ObjectReport {
        ok: false,
        kind,
        name,
        dns,
        provider,
        connection_examples,
        provider_actions: vec![
            "provider credentials are required before creating this object".to_owned(),
            "configure ~/.gumgum/providers/minio-main/credentials.json or run provider credential setup when available".to_owned(),
        ],
        reconciliation_steps: Vec::new(),
        message: "provider credentials are not configured".to_owned(),
    }
}

async fn daemon_configure_provider(
    State(state): State<DaemonState>,
    Json(request): Json<ProviderConfigureRequest>,
) -> Json<ProviderConfigureReport> {
    let config = gumgum_core::ProviderConfig::new(
        request.capability,
        request.kind,
        request.endpoint,
        request.vault,
    );
    let plan = provider_configure_plan((*state.graph_path).clone(), &config).await;
    let persist_result = ConfigStore::from_home_env()
        .and_then(|store| store.save_provider_config(&config))
        .and_then(|_| {
            GraphStore::new((*state.graph_path).clone()).materialize_provider(&DesiredProvider {
                name: config.provider.clone(),
                capability: config.capability,
            })
        });
    match persist_result {
        Ok(_) => {
            let mut steps = plan.unwrap_or_default();
            if steps.is_empty() {
                steps.push(GraphActionPlanner::ensure_provider_step(
                    gumgum_core::ProviderName::new(&config.provider).unwrap_or_else(|_| {
                        gumgum_core::ProviderName::new("provider.local").unwrap()
                    }),
                    config.capability,
                ));
            }
            match GraphActionExecutor::execute_steps(
                &steps,
                GraphExecutionContext {
                    graph_path: Some((*state.graph_path).clone()),
                    ..GraphExecutionContext::default()
                },
            )
            .await
            {
                Ok(actions) => Json(ProviderConfigureReport {
                    ok: true,
                    message: "provider configured and reconciled".to_owned(),
                    config: Some(config),
                    actions,
                    reconciliation_steps: steps,
                }),
                Err(error) => Json(ProviderConfigureReport {
                    ok: false,
                    message: error.to_report().message,
                    config: Some(config),
                    actions: Vec::new(),
                    reconciliation_steps: steps,
                }),
            }
        }
        Err(error) => Json(ProviderConfigureReport {
            ok: false,
            message: error.to_report().message,
            config: None,
            actions: Vec::new(),
            reconciliation_steps: plan.unwrap_or_default(),
        }),
    }
}

async fn provider_configure_plan(
    graph_path: PathBuf,
    config: &gumgum_core::ProviderConfig,
) -> Option<Vec<gumgum_core::GraphExecutionStep>> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let store = GraphStore::new(graph_path);
        let old_graph = store.load_desired_graph()?;
        let mut new_graph = old_graph.clone();
        new_graph.nodes.insert(DesiredGraphNode::Provider {
            name: gumgum_core::ProviderName::new(&config.provider)?,
            capability: config.capability,
        });
        Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
    })
    .await
    .ok()
    .and_then(Result::ok)
}

async fn daemon_providers() -> Json<ProviderStatusReport> {
    let providers = ProviderReconciler::statuses().await;
    Json(ProviderStatusReport {
        ok: true,
        message: format!("{} provider(s)", providers.len()),
        providers,
    })
}

async fn daemon_boot_default_providers() -> Json<ProviderBootReport> {
    let credentials = ConfigStore::from_home_env()
        .and_then(|store| store.load_or_init_default_provider_credentials());
    match credentials {
        Ok(credentials) => match ProviderReconciler::boot_defaults(&credentials).await {
            Ok(actions) => {
                let providers = ProviderReconciler::statuses().await;
                Json(ProviderBootReport {
                    ok: true,
                    message: "default providers booted".to_owned(),
                    actions,
                    providers,
                })
            }
            Err(error) => Json(ProviderBootReport {
                ok: false,
                actions: Vec::new(),
                providers: Vec::new(),
                message: error.to_report().message,
            }),
        },
        Err(error) => Json(ProviderBootReport {
            ok: false,
            actions: Vec::new(),
            providers: Vec::new(),
            message: error.to_report().message,
        }),
    }
}

async fn daemon_init_minio_credentials() -> Json<ProviderCredentialsInitReport> {
    let credentials = ConfigStore::from_home_env()
        .and_then(|store| store.load_or_init_default_provider_credentials());
    match credentials {
        Ok(credentials) => Json(ProviderCredentialsInitReport {
            ok: true,
            message: "default provider credentials configured".to_owned(),
            providers: credentials
                .into_iter()
                .map(|(provider, credentials)| provider_credentials_report(provider, credentials))
                .collect(),
        }),
        Err(error) => Json(ProviderCredentialsInitReport {
            ok: false,
            message: error.to_report().message,
            providers: Vec::new(),
        }),
    }
}

fn provider_credentials_report(
    provider: String,
    credentials: gumgum_core::ProviderCredentials,
) -> ProviderCredentialsReport {
    ProviderCredentialsReport {
        ok: true,
        provider,
        username_env: credentials.username_env,
        password_env: credentials.password_env,
        username: credentials.username,
        configured: true,
        message: "provider credentials configured".to_owned(),
    }
}

async fn daemon_env(
    State(state): State<DaemonState>,
    AxumPath(worker): AxumPath<String>,
) -> Json<EnvReport> {
    let path = (*state.graph_path).clone();
    let worker_for_task = worker.clone();
    let vars =
        tokio::task::spawn_blocking(move || GraphStore::new(path).binding_env(&worker_for_task))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, value)| EnvVar { name, value })
            .collect::<Vec<_>>();
    Json(EnvReport {
        ok: true,
        worker,
        message: format!("{} environment variable(s)", vars.len()),
        vars,
    })
}

async fn daemon_delete_binding(
    State(state): State<DaemonState>,
    Json(request): Json<BindingDeleteRequest>,
) -> Json<BindingReport> {
    let graph_path = (*state.graph_path).clone();
    let binding = WorkerBinding {
        capability: request.capability,
        object_name: request.object_name.clone(),
        worker: request.worker.clone(),
        binding: request.binding.clone(),
        access: "delete".to_owned(),
    };
    let reconciliation_steps = binding
        .delete_reconciliation_steps(graph_path.clone())
        .await;
    let deleted = if request.preview {
        false
    } else {
        let store = GraphStore::new(graph_path.clone());
        let binding_for_db = binding.clone();
        tokio::task::spawn_blocking(move || store.delete_binding(&binding_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    };
    let binding_actions = if request.preview {
        vec!["preview only; no bindings changed".to_owned()]
    } else {
        GraphActionExecutor::execute_steps(
            &reconciliation_steps,
            GraphExecutionContext {
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap_or_else(|error| {
            vec![format!(
                "binding delete reconcile failed: {}",
                error.to_report().message
            )]
        })
    };
    Json(BindingReport {
        ok: request.preview || deleted,
        object: format!("{}/{}", request.capability, request.object_name),
        worker: request.worker,
        binding: request.binding,
        binding_actions,
        reconciliation_steps,
        message: if request.preview {
            "binding delete preview".to_owned()
        } else if deleted {
            "binding deleted from graph".to_owned()
        } else {
            "binding was not present".to_owned()
        },
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
    let graph_path = (*state.graph_path).clone();
    let reconciliation_steps = request_for_db
        .reconciliation_steps(graph_path.clone())
        .await;
    let ok = tokio::task::spawn_blocking(move || store.materialize_binding(&request_for_db))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    let binding_actions = GraphActionExecutor::execute_steps(
        &reconciliation_steps,
        GraphExecutionContext {
            graph_path: Some(graph_path),
            ..GraphExecutionContext::default()
        },
    )
    .await
    .unwrap_or_else(|error| {
        vec![format!(
            "binding reconcile failed: {}",
            error.to_report().message
        )]
    });
    Json(BindingReport {
        ok,
        object: format!("{}/{}", request.capability, request.object_name),
        worker: request.worker,
        binding: request.binding,
        binding_actions,
        reconciliation_steps,
        message: "binding materialized in graph".to_owned(),
    })
}

#[derive(Debug, serde::Deserialize)]
struct RevisionsQuery {
    tail: Option<u32>,
    limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    limit: Option<u32>,
}

async fn daemon_events(
    State(state): State<DaemonState>,
    Query(query): Query<EventsQuery>,
) -> Json<EventsReport> {
    let path = (*state.graph_path).clone();
    let limit = query.limit.unwrap_or(50);
    let events =
        tokio::task::spawn_blocking(move || GraphStore::new(path).list_reconcile_events(limit))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
    Json(EventsReport {
        ok: true,
        message: format!("{} reconciliation event(s)", events.len()),
        events,
    })
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
            let graph_path = (*state.graph_path).clone();
            let mut steps = deploy.reconciliation_steps(graph_path.clone()).await;
            if steps.is_empty() {
                steps.push(deploy.execution_step());
            }
            let store = GraphStore::new(graph_path.clone());
            let deploy_for_db = deploy.clone();
            let _ =
                tokio::task::spawn_blocking(move || store.materialize_deploy(&deploy_for_db)).await;
            let mut actions = GraphActionExecutor::execute_steps(
                &steps,
                GraphExecutionContext {
                    object_plan: None,
                    provider_credentials: None,
                    graph_path: Some(graph_path),
                },
            )
            .await
            .unwrap_or_else(|error| {
                vec![format!(
                    "rollback reconcile failed: {}",
                    error.to_report().message
                )]
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

async fn daemon_delete_deploy(
    State(state): State<DaemonState>,
    Json(request): Json<DeploymentDeleteRequest>,
) -> Json<DeployApplyReport> {
    let graph_path = (*state.graph_path).clone();
    let worker = request.worker.clone();
    let desired = tokio::task::spawn_blocking({
        let graph_path = graph_path.clone();
        let worker = worker.clone();
        move || GraphStore::new(graph_path).desired_deploy(&worker)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    let Some(deploy) = desired else {
        return Json(DeployApplyReport {
            ok: request.preview,
            worker: request.worker,
            materialized: false,
            changed: false,
            actions: vec!["deployment was not present".to_owned()],
            reconciliation_steps: Vec::new(),
            message: "deployment was not present".to_owned(),
        });
    };
    let reconciliation_steps = deploy.delete_reconciliation_steps(graph_path.clone()).await;
    let deleted = if request.preview {
        false
    } else {
        let graph_path = graph_path.clone();
        let worker = request.worker.clone();
        tokio::task::spawn_blocking(move || GraphStore::new(graph_path).delete_deploy(&worker))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    };
    let actions = if request.preview {
        vec!["preview only; no deployments changed".to_owned()]
    } else {
        GraphActionExecutor::execute_steps(
            &reconciliation_steps,
            GraphExecutionContext {
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap_or_else(|error| {
            vec![format!(
                "deployment delete reconcile failed: {}",
                error.to_report().message
            )]
        })
    };
    Json(DeployApplyReport {
        ok: request.preview || deleted,
        worker: request.worker,
        materialized: !deleted,
        changed: deleted,
        actions,
        reconciliation_steps,
        message: if request.preview {
            "deployment delete preview".to_owned()
        } else if deleted {
            "deployment deleted from graph".to_owned()
        } else {
            "deployment was not present".to_owned()
        },
    })
}

async fn daemon_deploy(
    State(state): State<DaemonState>,
    Json(request): Json<DeployRequest>,
) -> Json<DeployApplyReport> {
    let path = (*state.graph_path).clone();
    let reconcile_path = path.clone();
    let store = GraphStore::new(path.clone());
    let mut reconciliation_steps = deploy_reconciliation_plan(path.clone(), &request).await;
    let request_for_db = request.clone().into_desired_deploy();
    let deploy_for_db = request_for_db.clone();
    let materialized =
        tokio::task::spawn_blocking(move || store.materialize_deploy(&deploy_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
    if reconciliation_steps.is_empty() {
        reconciliation_steps.push(request_for_db.execution_step());
    }
    let deploy_context = GraphExecutionContext {
        object_plan: None,
        provider_credentials: None,
        graph_path: Some(reconcile_path),
    };
    let actions = GraphActionExecutor::execute_steps(&reconciliation_steps, deploy_context)
        .await
        .unwrap_or_else(|error| vec![format!("reconcile failed: {}", error.to_report().message)]);
    let changed = actions.iter().any(|action| {
        action.starts_with("pull ")
            || action.starts_with("recreate ")
            || action.starts_with("project ")
    });
    Json(DeployApplyReport {
        ok: materialized,
        worker: request.worker,
        materialized,
        changed,
        actions,
        reconciliation_steps,
        message: "desired deployment materialized and reconciled".to_owned(),
    })
}

trait DeployRequestExt {
    fn into_desired_deploy(self) -> DesiredDeploy;
}

impl DeployRequestExt for DeployRequest {
    fn into_desired_deploy(self) -> DesiredDeploy {
        DesiredDeploy {
            worker: self.worker,
            image: self.image,
            container: self.container,
            route: self.route,
            port: self.port,
            health: self.health,
        }
    }
}

async fn deploy_reconciliation_plan(
    graph_path: PathBuf,
    request: &DeployRequest,
) -> Vec<gumgum_core::GraphExecutionStep> {
    let request = request.clone();
    tokio::task::spawn_blocking(move || {
        let store = GraphStore::new(graph_path);
        let old_graph = store.load_desired_graph()?;
        let mut new_graph = old_graph.clone();
        new_graph
            .nodes
            .insert(request.into_desired_deploy().graph_node()?);
        Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default()
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

    #[tokio::test]
    async fn daemon_events_lists_newest_reconciliation_events() {
        let path = temp_graph_path("events-list");
        let store = GraphStore::new(path.clone());
        store
            .record_reconcile_event(&gumgum_core::NewReconcileEvent {
                kind: gumgum_core::ControlPlaneEventKind::Reconciliation,
                status: gumgum_core::ReconcileEventStatus::Planned,
                target: "provider/manual.main".to_owned(),
                action: "ensure_provider".to_owned(),
                message: "plan provider".to_owned(),
            })
            .unwrap();
        store
            .record_reconcile_event(&gumgum_core::NewReconcileEvent {
                kind: gumgum_core::ControlPlaneEventKind::Reconciliation,
                status: gumgum_core::ReconcileEventStatus::Executed,
                target: "provider/manual.main".to_owned(),
                action: "ensure_provider".to_owned(),
                message: "configured provider".to_owned(),
            })
            .unwrap();
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(report) = daemon_events(State(state), Query(EventsQuery { limit: Some(1) })).await;

        assert!(report.ok);
        assert_eq!(report.message, "1 reconciliation event(s)");
        assert_eq!(report.events.len(), 1);
        assert_eq!(
            report.events[0].status,
            gumgum_core::ReconcileEventStatus::Executed
        );
        assert_eq!(report.events[0].target, "provider/manual.main");
        assert_eq!(report.events[0].message, "configured provider");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn daemon_version_report_advertises_rollback_revision_capabilities() {
        let report = daemon_version_report();

        assert!(report.ok);
        assert!(
            report
                .capabilities
                .contains(&"rollback_revisions".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"rollback_revision_id".to_owned())
        );
    }

    #[test]
    fn missing_provider_credentials_report_blocks_object_creation() {
        let report = missing_provider_credentials_report(
            "blob".to_owned(),
            "uploads".to_owned(),
            "uploads.blob.example.test".to_owned(),
            "minio.main".to_owned(),
            Vec::new(),
        );

        assert!(!report.ok);
        assert_eq!(report.message, "provider credentials are not configured");
        assert!(
            report
                .provider_actions
                .iter()
                .any(|action| action.contains("provider credentials are required"))
        );
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
    fn rollback_deploy_step_uses_self_contained_deploy_runtime_target() {
        let deploy = DesiredDeploy {
            worker: "api".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: "api.example.test".to_owned(),
            port: 3000,
            health: "/healthz".to_owned(),
        };

        let step = deploy.execution_step();

        assert!(matches!(
            step.target,
            gumgum_core::GraphExecutionTarget::DeployRuntime {
                worker: Some(ref worker),
                ref container,
                ref image,
                route: Some(ref route),
                port: Some(port),
                health: Some(ref health),
            } if worker.as_str() == "api"
                && container.as_str() == "gumgum-api"
                && image.as_str() == "registry/api:1"
                && route.as_str() == "api.example.test"
                && port.get() == 3000
                && health.as_str() == "/healthz"
        ));
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
