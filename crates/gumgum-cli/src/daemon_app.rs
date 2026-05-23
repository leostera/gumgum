use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::header::CONTENT_TYPE,
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gumgum_api::{
    AffectedReport, BindingDeleteRequest, BindingReport, BindingRequest, BucketObjectReport,
    BucketObjectRequest, DaemonVersionReport, DeployApplyReport, DeployRequest,
    DeploymentDeleteRequest, DeploymentRevisionDeleteReport, DeploymentRevisionsReport,
    DomainAddRequest, DomainReport, EnvReport, EnvVar, EventsReport, GraphNode, GraphReport,
    LogsReport, ObjectDeleteRequest, ObjectReport, ObjectRequest, ProviderBootReport,
    ProviderConfigureReport, ProviderConfigureRequest, ProviderCredentialsInitReport,
    ProviderCredentialsReport, ProviderStatusReport, RollbackReport, RollbackRequest,
};
use gumgum_core::{
    ConfigStore, DesiredDeploy, DesiredProvider, DomainRecord, ErrorCode, GlobalObject,
    GraphActionExecutor, GraphActionPlanner, GraphExecutionContext, GraphStore, GumgumError,
    LocalPlatform, ProviderReconciler, Subsystem, WorkerBinding, affected_subgraph, internal_db,
    not_configured_status, object_dns, object_provider_plan, render_mermaid_graph,
};
use std::{convert::Infallible, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{process::Command as TokioCommand, sync::mpsc};

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
        eprintln!("Starting gumgumd, the GumGum control-plane daemon.");
        eprintln!(
            "Most users do not need to run this manually; prefer `gumgum setup` or `gumgum server add`."
        );
        eprintln!("Preparing local Docker/network prerequisites...");
        LocalPlatform::ensure(false).await?;
        let graph_path = ConfigStore::from_home_env()?.root().join("graph.sqlite");
        eprintln!("Using graph store: {}", graph_path.display());
        internal_db::migrate_graph_store(&graph_path).await?;
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
        eprintln!("gumgumd listening on http://{addr}");
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
            .route("/v0/deploy/stream", post(daemon_deploy_stream))
            .route("/v0/rollback", post(daemon_rollback))
            .route("/v0/revisions/{worker}", get(daemon_revisions))
            .route(
                "/v0/revisions/{worker}/{revision_id}",
                axum::routing::delete(daemon_delete_revision),
            )
            .route("/v0/events", get(daemon_events))
            .route(
                "/v0/objects",
                post(daemon_create_object).delete(daemon_delete_object),
            )
            .route(
                "/v0/bindings",
                post(daemon_create_binding).delete(daemon_delete_binding),
            )
            .route("/v0/domains", post(daemon_add_domain))
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
            .route("/v0/buckets/{action}", post(daemon_bucket_object_action))
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
            "gumgum:events".to_owned(),
            "gumgum:rollback:revisions".to_owned(),
            "gumgum:rollback:revision_id".to_owned(),
            "gumgum:rollback:revision_delete".to_owned(),
            "gumgum:objects:create_preview".to_owned(),
            "gumgum:bindings:create_preview".to_owned(),
            "gumgum:bindings:delete".to_owned(),
            "gumgum:objects:delete".to_owned(),
            "gumgum:deployments:delete".to_owned(),
            "gumgum:deployments:stream".to_owned(),
            "gumgum:buckets:objects".to_owned(),
        ],
    }
}

async fn status() -> Json<gumgum_core::StatusReport> {
    Json(not_configured_status())
}

async fn daemon_add_domain(Json(request): Json<DomainAddRequest>) -> Json<DomainReport> {
    let mut actions = vec![format!(
        "record domain {} with {} provider",
        request.name,
        request.provider.as_str()
    )];
    let store = match ConfigStore::from_home_env() {
        Ok(store) => store,
        Err(error) => {
            return Json(DomainReport {
                ok: false,
                name: request.name,
                provider: request.provider,
                ingress: request.ingress,
                actions: vec![error.to_report().message],
                message: "domain was not saved".to_owned(),
            });
        }
    };
    if request.provider == gumgum_core::DomainProvider::Cloudflare {
        if let Some(grant) = request.cloudflare_grant {
            if let Err(error) = gumgum_core::cloudflare::api::CloudflareClient::new(&grant)
                .validate_zone_access(&request.name)
                .await
            {
                return Json(DomainReport {
                    ok: false,
                    name: request.name,
                    provider: request.provider,
                    ingress: request.ingress,
                    actions: vec![format!(
                        "Cloudflare zone verification failed: {}",
                        error.to_report().message
                    )],
                    message: "Cloudflare grant was not saved".to_owned(),
                });
            }
            actions.push(format!(
                "verified Cloudflare zone access for {} using provided grant",
                request.name
            ));
            if let Err(error) = store.save_cloudflare_grant(&grant) {
                return Json(DomainReport {
                    ok: false,
                    name: request.name,
                    provider: request.provider,
                    ingress: request.ingress,
                    actions: vec![error.to_report().message],
                    message: "Cloudflare grant was not saved".to_owned(),
                });
            }
            actions.push("saved Cloudflare grant on server".to_owned());
        } else if let Ok(Some(grant)) = store.load_cloudflare_grant() {
            match gumgum_core::cloudflare::api::CloudflareClient::new(&grant)
                .validate_zone_access(&request.name)
                .await
            {
                Ok(()) => actions.push(format!(
                    "verified Cloudflare zone access for {} using saved grant",
                    request.name
                )),
                Err(error) => {
                    return Json(DomainReport {
                        ok: false,
                        name: request.name,
                        provider: request.provider,
                        ingress: request.ingress,
                        actions: vec![format!(
                            "Cloudflare zone verification failed: {}",
                            error.to_report().message
                        )],
                        message: "domain was not saved".to_owned(),
                    });
                }
            }
        } else {
            return Json(DomainReport {
                ok: false,
                name: request.name,
                provider: request.provider,
                ingress: request.ingress,
                actions: vec!["Cloudflare grant is required".to_owned()],
                message: "domain was not saved".to_owned(),
            });
        }
    }
    let record = DomainRecord {
        name: request.name.clone(),
        provider: request.provider,
        ingress: request.ingress,
    };
    let ok = store.save_domain(record).is_ok();
    if request.ingress == gumgum_core::IngressMode::Cloudflare {
        actions.push("Cloudflare tunnel ingress will be converged for this domain".to_owned());
    }
    Json(DomainReport {
        ok,
        name: request.name,
        provider: request.provider,
        ingress: request.ingress,
        actions,
        message: if ok {
            "domain saved".to_owned()
        } else {
            "domain was not saved".to_owned()
        },
    })
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
    let provider_plan = object_provider_plan(request.capability, &request.name, &dns);
    let provider_credentials = required_provider_credentials(request.capability).unwrap_or(None);
    let existing_bindings = tokio::task::spawn_blocking({
        let graph_path = graph_path.clone();
        let object = object.clone();
        move || GraphStore::new(graph_path).object_bindings(&object)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default();
    if !existing_bindings.is_empty() {
        return Json(ObjectReport {
            ok: false,
            kind: capability_name,
            name: request.name,
            dns,
            provider,
            connection_examples: Vec::new(),
            provider_actions: existing_bindings
                .iter()
                .map(|binding| {
                    format!(
                        "still bound to worker {} as {}",
                        binding.worker, binding.binding
                    )
                })
                .collect(),
            reconciliation_steps: Vec::new(),
            typed_events: Vec::new(),
            message: "object has active bindings; unbind it before deleting".to_owned(),
        });
    }
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
    let (provider_actions, typed_events) = if request.preview {
        (
            vec!["preview only; no objects changed".to_owned()],
            Vec::new(),
        )
    } else {
        match GraphActionExecutor::execute_steps_report(
            &reconciliation_steps,
            #[allow(clippy::needless_update)]
            GraphExecutionContext {
                object_plan: Some(provider_plan.clone()),
                provider_credentials,
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        {
            Ok(report) => (report.actions, report.typed_events),
            Err(error) => (
                vec![format!(
                    "object delete reconcile failed: {}",
                    error.to_report().message
                )],
                Vec::new(),
            ),
        }
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
        typed_events,
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
    let mut provider_plan = object_provider_plan(capability, &request.name, &dns);
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
    provider_plan.object_password = db_password.clone();
    let provider = provider_plan.provider.provider.clone();
    let reconciliation_steps = request_for_db
        .reconciliation_steps((*state.graph_path).clone())
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
    let secret_object_name = format!("{}-password", gumgum_core::sanitize_name(&request.name));
    let object_name_for_secret = request.name.clone();
    let ok = if request.preview {
        true
    } else {
        tokio::task::spawn_blocking(move || {
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
        .unwrap_or(false)
    };
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
    let (provider_actions, typed_events) = if request.preview {
        (
            vec!["preview only; no objects changed".to_owned()],
            Vec::new(),
        )
    } else {
        match GraphActionExecutor::execute_steps_report(
            &execution_steps,
            GraphExecutionContext {
                object_plan: Some(provider_plan.clone()),
                provider_credentials,
                graph_path: Some((*state.graph_path).clone()),
                event_sender: None,
            },
        )
        .await
        {
            Ok(report) => (report.actions, report.typed_events),
            Err(error) => (
                vec![format!(
                    "provider reconcile failed: {}",
                    error.to_report().message
                )],
                Vec::new(),
            ),
        }
    };
    let reconcile_ok = !provider_actions
        .iter()
        .any(|action| action.starts_with("provider reconcile failed:"));
    Json(ObjectReport {
        ok: ok && reconcile_ok,
        kind: capability_name,
        name: request.name,
        dns,
        provider,
        connection_examples: provider_plan.connection_examples,
        provider_actions,
        reconciliation_steps: execution_steps,
        typed_events,
        message: if request.preview {
            "object create preview".to_owned()
        } else if capability == gumgum_core::Capability::Db {
            "global object materialized with managed password secret and provider reconciled"
                .to_owned()
        } else {
            "global object materialized and provider reconciled".to_owned()
        },
    })
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
        typed_events: Vec::new(),
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
    Some(
        DesiredProvider {
            name: config.provider.clone(),
            capability: config.capability,
        }
        .reconciliation_steps(graph_path)
        .await,
    )
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
    let (binding_actions, typed_events) = if request.preview {
        (
            vec!["preview only; no bindings changed".to_owned()],
            Vec::new(),
        )
    } else {
        match GraphActionExecutor::execute_steps_report(
            &reconciliation_steps,
            GraphExecutionContext {
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        {
            Ok(report) => (report.actions, report.typed_events),
            Err(error) => (
                vec![format!(
                    "binding delete reconcile failed: {}",
                    error.to_report().message
                )],
                Vec::new(),
            ),
        }
    };
    Json(BindingReport {
        ok: request.preview || deleted,
        object: format!("{}/{}", request.capability, request.object_name),
        worker: request.worker,
        binding: request.binding,
        binding_actions,
        reconciliation_steps,
        typed_events,
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
    let ok = if request.preview {
        true
    } else {
        tokio::task::spawn_blocking(move || store.materialize_binding(&request_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    };
    let (binding_actions, typed_events) = if request.preview {
        (
            vec!["preview only; no bindings changed".to_owned()],
            Vec::new(),
        )
    } else {
        match GraphActionExecutor::execute_steps_report(
            &reconciliation_steps,
            GraphExecutionContext {
                graph_path: Some(graph_path),
                ..GraphExecutionContext::default()
            },
        )
        .await
        {
            Ok(report) => (report.actions, report.typed_events),
            Err(error) => (
                vec![format!(
                    "binding reconcile failed: {}",
                    error.to_report().message
                )],
                Vec::new(),
            ),
        }
    };
    Json(BindingReport {
        ok,
        object: format!("{}/{}", request.capability, request.object_name),
        worker: request.worker,
        binding: request.binding,
        binding_actions,
        reconciliation_steps,
        typed_events,
        message: if request.preview {
            "binding create preview".to_owned()
        } else {
            "binding materialized in graph".to_owned()
        },
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
    let typed_events = events.iter().cloned().map(Into::into).collect();
    Json(EventsReport {
        ok: true,
        message: format!("{} reconciliation event(s)", events.len()),
        typed_events,
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
    let (current, revisions) = tokio::task::spawn_blocking(move || {
        let store = GraphStore::new(path);
        let current = store.desired_deploy(&worker_for_task)?;
        let revisions = store.deployment_revisions(&worker_for_task, limit)?;
        Ok::<_, gumgum_core::GumgumError>((current, revisions))
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default();
    Json(DeploymentRevisionsReport {
        ok: true,
        worker,
        current,
        message: format!("{} deployment revision(s)", revisions.len()),
        revisions,
    })
}

async fn daemon_delete_revision(
    State(state): State<DaemonState>,
    AxumPath((worker, revision_id)): AxumPath<(String, i64)>,
) -> Json<DeploymentRevisionDeleteReport> {
    let path = (*state.graph_path).clone();
    let worker_for_task = worker.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        GraphStore::new(path).delete_deployment_revision(&worker_for_task, revision_id)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(false);
    Json(DeploymentRevisionDeleteReport {
        ok: deleted,
        worker,
        revision_id,
        deleted,
        actions: if deleted {
            vec![
                format!("deleted deployment revision {revision_id}"),
                "no containers or desired deployments changed".to_owned(),
            ]
        } else {
            vec![format!("deployment revision {revision_id} not found")]
        },
        message: if deleted {
            format!("deleted deployment revision {revision_id}")
        } else {
            format!("deployment revision {revision_id} not found")
        },
    })
}

#[derive(Debug, serde::Deserialize)]
struct LogsQuery {
    tail: Option<u32>,
}

async fn daemon_bucket_object_action(
    AxumPath(action): AxumPath<String>,
    Json(request): Json<BucketObjectRequest>,
) -> Json<BucketObjectReport> {
    if !matches!(action.as_str(), "ls" | "get" | "rm" | "cp" | "sync") {
        return Json(BucketObjectReport {
            ok: false,
            action: action.clone(),
            bucket: request.bucket,
            path: request.path,
            objects: Vec::new(),
            content: None,
            content_base64: None,
            actions: Vec::new(),
            message: format!("unknown bucket object action {action}"),
        });
    }
    let credentials = match ConfigStore::from_home_env()
        .and_then(|store| store.load_provider_credentials("minio.main"))
    {
        Ok(Some(credentials)) => credentials,
        Ok(None) => {
            return Json(BucketObjectReport {
                ok: false,
                action,
                bucket: request.bucket,
                path: request.path,
                objects: Vec::new(),
                content: None,
                content_base64: None,
                actions: vec!["configure minio.main provider credentials".to_owned()],
                message: "minio provider credentials are not configured".to_owned(),
            });
        }
        Err(error) => {
            return Json(BucketObjectReport {
                ok: false,
                action,
                bucket: request.bucket,
                path: request.path,
                objects: Vec::new(),
                content: None,
                content_base64: None,
                actions: vec![error.to_string()],
                message: "could not load minio provider credentials".to_owned(),
            });
        }
    };
    let bucket = request.bucket.clone().unwrap_or_default();
    let path = request.path.clone().unwrap_or_default();
    let result = match action.as_str() {
        "ls" => gumgum_core::providers::minio::list_objects(
            &bucket,
            request.path.as_deref(),
            &credentials,
        )
        .await
        .map(|objects| BucketObjectReport {
            ok: true,
            action: action.clone(),
            bucket: request.bucket.clone(),
            path: request.path.clone(),
            objects,
            content: None,
            content_base64: None,
            actions: Vec::new(),
            message: "bucket objects listed".to_owned(),
        }),
        "get" => gumgum_core::providers::minio::get_object_bytes(&bucket, &path, &credentials)
            .await
            .map(|bytes| BucketObjectReport {
                ok: true,
                action: action.clone(),
                bucket: request.bucket.clone(),
                path: request.path.clone(),
                objects: Vec::new(),
                content: String::from_utf8(bytes.clone()).ok(),
                content_base64: Some(BASE64.encode(bytes)),
                actions: Vec::new(),
                message: "bucket object read".to_owned(),
            }),
        "rm" => gumgum_core::providers::minio::remove_object(&bucket, &path, &credentials)
            .await
            .map(|actions| BucketObjectReport {
                ok: true,
                action: action.clone(),
                bucket: request.bucket.clone(),
                path: request.path.clone(),
                objects: Vec::new(),
                content: None,
                content_base64: None,
                actions,
                message: "bucket object removed".to_owned(),
            }),
        "cp" => match request.content_base64 {
            Some(content) => match BASE64.decode(content) {
                Ok(bytes) => match split_bucket_object_path(
                    request.destination.as_deref().unwrap_or_default(),
                ) {
                    Ok((bucket, path)) => {
                        gumgum_core::providers::minio::put_object(
                            &bucket,
                            &path,
                            bytes,
                            &credentials,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                },
                Err(source) => Err(GumgumError::structured(
                    Subsystem::Api,
                    ErrorCode::InvalidArgs,
                    "bucket upload content is not valid base64",
                )
                .likely_cause(source.to_string())
                .build()),
            },
            None => {
                gumgum_core::providers::minio::copy_object(
                    request.source.as_deref().unwrap_or_default(),
                    request.destination.as_deref().unwrap_or_default(),
                    &credentials,
                )
                .await
            }
        }
        .map(|actions| BucketObjectReport {
            ok: true,
            action: action.clone(),
            bucket: request.bucket.clone(),
            path: request.path.clone(),
            objects: Vec::new(),
            content: None,
            content_base64: None,
            actions,
            message: "bucket object copied".to_owned(),
        }),
        "sync" => gumgum_core::providers::minio::sync_objects(
            request.source.as_deref().unwrap_or_default(),
            request.destination.as_deref().unwrap_or_default(),
            &credentials,
        )
        .await
        .map(|actions| BucketObjectReport {
            ok: true,
            action: action.clone(),
            bucket: request.bucket.clone(),
            path: request.path.clone(),
            objects: Vec::new(),
            content: None,
            content_base64: None,
            actions,
            message: "bucket objects synced".to_owned(),
        }),
        _ => unreachable!("bucket object action was validated before provider dispatch"),
    };
    Json(result.unwrap_or_else(|error| BucketObjectReport {
        ok: false,
        action,
        bucket: request.bucket,
        path: request.path,
        objects: Vec::new(),
        content: None,
        content_base64: None,
        actions: vec![error.to_string()],
        message: "bucket object operation failed".to_owned(),
    }))
}

fn split_bucket_object_path(value: &str) -> gumgum_core::Result<(String, String)> {
    let (bucket, path) = value
        .trim_start_matches('/')
        .split_once('/')
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::InvalidArgs,
                format!("bucket object path must be bucket/path: {value}"),
            )
            .build()
        })?;
    if bucket.is_empty() || path.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::InvalidArgs,
            format!("bucket object path must be bucket/path: {value}"),
        )
        .build());
    }
    Ok((bucket.to_owned(), path.to_owned()))
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
    let rollback_target = tokio::task::spawn_blocking(move || {
        let revision = store.rollback_revision(&worker, revision_id)?;
        let current = store.desired_deploy(&worker)?;
        Ok::<_, gumgum_core::GumgumError>((revision, current))
    })
    .await
    .ok()
    .and_then(Result::ok);
    if let Some((Some(revision), current)) = rollback_target {
        let deploy = revision.deploy.clone();
        let image = deploy.image.clone();
        let route_warning = rollback_route_warning(current.as_ref(), &deploy);
        let actions = if request.preview {
            rollback_preview_actions(&image, route_warning)
        } else {
            let graph_path = (*state.graph_path).clone();
            let mut steps = deploy.reconciliation_steps(graph_path.clone()).await;
            if steps.is_empty() {
                steps.push(deploy.execution_step());
            }
            let execution_result = GraphActionExecutor::execute_steps(
                &steps,
                GraphExecutionContext {
                    object_plan: None,
                    provider_credentials: None,
                    graph_path: Some(graph_path.clone()),
                    event_sender: None,
                },
            )
            .await;
            let mut actions = match execution_result {
                Ok(actions) => {
                    let store = GraphStore::new(graph_path);
                    let deploy_for_db = deploy.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.materialize_deploy(&deploy_for_db)
                    })
                    .await;
                    actions
                }
                Err(error) => vec![format!(
                    "rollback reconcile failed: {}; desired deploy was not changed",
                    error.to_report().message
                )],
            };
            if let Some(warning) = route_warning {
                actions.insert(0, warning);
            }
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

fn rollback_preview_actions(image: &str, route_warning: Option<String>) -> Vec<String> {
    let mut actions = vec![format!("would rollback to {image}")];
    if let Some(warning) = route_warning {
        actions.push(warning);
    }
    actions.push("preview only; no containers changed".to_owned());
    actions
}

fn rollback_route_warning(
    current: Option<&gumgum_core::DesiredDeploy>,
    target: &gumgum_core::DesiredDeploy,
) -> Option<String> {
    current.and_then(|current| {
        if current.route == target.route {
            None
        } else {
            Some(format!(
                "warning: rollback would change route from {} to {}",
                current.route.as_deref().unwrap_or("<none>"),
                target.route.as_deref().unwrap_or("<none>")
            ))
        }
    })
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
        route: revision.deploy.route,
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
        let worker = request.worker.clone();
        return Json(DeployApplyReport {
            ok: request.preview,
            worker: request.worker,
            materialized: false,
            changed: false,
            actions: vec!["deployment was not present".to_owned()],
            reconciliation_steps: Vec::new(),
            typed_events: vec![gumgum_core::GumgumEvent::DeploymentFailed {
                worker: logical_deployment_worker(&worker).to_owned(),
                environment: deployment_env(&worker),
                error: "deployment was not present".to_owned(),
            }],
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
    let ok = request.preview || deleted;
    let message = if request.preview {
        "deployment delete preview".to_owned()
    } else if deleted {
        "deployment deleted from graph".to_owned()
    } else {
        "deployment was not present".to_owned()
    };
    let typed_events = deploy_apply_typed_events(
        &request.worker,
        None,
        &reconciliation_steps,
        &[],
        &actions,
        ok,
        &message,
    );
    Json(DeployApplyReport {
        ok,
        worker: request.worker,
        materialized: !deleted,
        changed: deleted,
        actions,
        reconciliation_steps,
        typed_events,
        message: if request.preview {
            "deployment delete preview".to_owned()
        } else if deleted {
            "deployment deleted from graph".to_owned()
        } else {
            "deployment was not present".to_owned()
        },
    })
}

async fn daemon_deploy_stream(
    State(state): State<DaemonState>,
    Json(request): Json<DeployRequest>,
) -> impl IntoResponse {
    let (sender, receiver) = mpsc::unbounded_channel::<gumgum_core::GumgumEvent>();
    tokio::spawn(async move {
        let report = daemon_deploy_report(state, request, Some(sender.clone())).await;
        for event in report
            .typed_events
            .into_iter()
            .filter(|event| !deploy_stream_sends_live(event))
        {
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    typed_event_stream_response(receiver)
}

fn typed_event_stream_response(
    receiver: mpsc::UnboundedReceiver<gumgum_core::GumgumEvent>,
) -> impl IntoResponse {
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        let event = receiver.recv().await?;
        let mut line = serde_json::to_vec(&event).ok()?;
        line.push(b'\n');
        Some((Ok::<_, Infallible>(line), receiver))
    });
    (
        [(CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(stream),
    )
}

fn deploy_stream_sends_live(event: &gumgum_core::GumgumEvent) -> bool {
    matches!(
        event,
        gumgum_core::GumgumEvent::DeploymentStarted { .. }
            | gumgum_core::GumgumEvent::ReconcileStepPlanned { .. }
            | gumgum_core::GumgumEvent::ReconcileStepExecuted { .. }
            | gumgum_core::GumgumEvent::ReconcileStepFailed { .. }
    )
}

#[cfg(test)]
fn typed_events_ndjson(events: &[gumgum_core::GumgumEvent]) -> String {
    let mut body = String::new();
    for event in events {
        if let Ok(line) = serde_json::to_string(event) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    body
}

async fn daemon_deploy(
    State(state): State<DaemonState>,
    Json(request): Json<DeployRequest>,
) -> Json<DeployApplyReport> {
    Json(daemon_deploy_report(state, request, None).await)
}

async fn daemon_deploy_report(
    state: DaemonState,
    request: DeployRequest,
    event_sender: Option<mpsc::UnboundedSender<gumgum_core::GumgumEvent>>,
) -> DeployApplyReport {
    let path = (*state.graph_path).clone();
    let reconcile_path = path.clone();
    let store = GraphStore::new(path.clone());
    let previous_route = {
        let path = path.clone();
        let worker = request.worker.clone();
        tokio::task::spawn_blocking(move || {
            GraphStore::new(path)
                .desired_deploy(&worker)
                .map(|deploy| deploy.and_then(|deploy| deploy.route))
        })
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
    };
    if request.publish {
        if let Some(route) = &request.route {
            match ConfigStore::from_home_env().and_then(|store| store.load_domains()) {
                Ok(domains) if !managed_domain_matches(&domains, route) => {
                    let worker = request.worker.clone();
                    return DeployApplyReport {
                        ok: false,
                        worker: request.worker,
                        materialized: false,
                        changed: false,
                        actions: vec![format!(
                            "publish DNS failed: no managed domain matches published route {route} (add the domain to this server before deploying a published route)"
                        )],
                        reconciliation_steps: Vec::new(),
                        typed_events: vec![gumgum_core::GumgumEvent::DeploymentFailed {
                            worker: logical_deployment_worker(&worker).to_owned(),
                            environment: deployment_env(&worker),
                            error: "published route domain is not registered on this server".to_owned(),
                        }],
                        message: "deployment blocked before reconciliation; published route domain is not registered on this server".to_owned(),
                    };
                }
                Err(error) => {
                    let worker = request.worker.clone();
                    return DeployApplyReport {
                        ok: false,
                        worker: request.worker,
                        materialized: false,
                        changed: false,
                        actions: vec![format!(
                            "publish DNS failed: {}",
                            error.to_report().message
                        )],
                        reconciliation_steps: Vec::new(),
                        typed_events: vec![gumgum_core::GumgumEvent::DeploymentFailed {
                            worker: logical_deployment_worker(&worker).to_owned(),
                            environment: deployment_env(&worker),
                            error: "domain configuration could not be loaded".to_owned(),
                        }],
                        message: "deployment blocked before reconciliation; domain configuration could not be loaded".to_owned(),
                    };
                }
                _ => {}
            }
        }
    }
    let mut reconciliation_steps = deploy_reconciliation_plan(path.clone(), &request).await;
    let request_for_db = request.clone().into_desired_deploy();
    if reconciliation_steps.is_empty() {
        reconciliation_steps.push(request_for_db.execution_step());
    }
    if let Some(sender) = &event_sender {
        let _ = sender.send(gumgum_core::GumgumEvent::DeploymentStarted {
            worker: logical_deployment_worker(&request.worker).to_owned(),
            environment: deployment_env(&request.worker),
            image: request.image.clone(),
        });
    }
    let deploy_context = GraphExecutionContext {
        object_plan: None,
        provider_credentials: None,
        graph_path: Some(reconcile_path),
        event_sender: event_sender.clone(),
    };
    let (mut actions, execution_typed_events) = match GraphActionExecutor::execute_steps_report(
        &reconciliation_steps,
        deploy_context,
    )
    .await
    {
        Ok(report) => (report.actions, report.typed_events),
        Err(error) => (
            vec![format!("reconcile failed: {}", error.to_report().message)],
            Vec::new(),
        ),
    };
    let reconcile_ok = !actions
        .iter()
        .any(|action| action.starts_with("reconcile failed:"));
    let materialize_changed = if reconcile_ok {
        let deploy_for_db = request_for_db.clone();
        match tokio::task::spawn_blocking(move || store.materialize_deploy(&deploy_for_db)).await {
            Ok(Ok(changed)) => Some(changed),
            _ => None,
        }
    } else {
        actions.push("desired deployment was not changed".to_owned());
        None
    };
    let materialized = materialize_changed.is_some();
    let mut publish_ok = true;
    if materialized && request.publish {
        if let Some(route) = &request.route {
            match ConfigStore::from_home_env()
                .and_then(|store| Ok((store.load_domains()?, store.load_cloudflare_grant()?)))
            {
                Ok((domains, Some(grant))) => {
                    match gumgum_core::cloudflare::dns::ensure_published_route(
                        &domains, &grant, route,
                    )
                    .await
                    {
                        Ok(mut dns_actions) => {
                            actions.append(&mut dns_actions);
                            if let Some(previous_route) = previous_route.as_deref() {
                                if previous_route != route {
                                    match gumgum_core::cloudflare::dns::delete_published_route(
                                        &domains,
                                        &grant,
                                        previous_route,
                                    )
                                    .await
                                    {
                                        Ok(mut cleanup_actions) => {
                                            actions.append(&mut cleanup_actions)
                                        }
                                        Err(error) => {
                                            let report = error.to_report();
                                            let cause = report
                                                .likely_cause
                                                .map(|cause| format!(" ({cause})"))
                                                .unwrap_or_default();
                                            actions.push(format!(
                                                "publish DNS cleanup failed: {}{}",
                                                report.message, cause
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            publish_ok = false;
                            let report = error.to_report();
                            let cause = report
                                .likely_cause
                                .map(|cause| format!(" ({cause})"))
                                .unwrap_or_default();
                            actions
                                .push(format!("publish DNS failed: {}{}", report.message, cause));
                        }
                    }
                }
                Ok((_domains, None)) => {
                    publish_ok = false;
                    actions.push(format!(
                        "publish DNS failed for {route}; no Cloudflare grant saved on server"
                    ));
                }
                Err(error) => {
                    publish_ok = false;
                    actions.push(format!("publish DNS failed: {}", error.to_report().message));
                }
            }
        }
    }
    let changed = materialize_changed.unwrap_or(false)
        || actions.iter().any(|action| {
            action.starts_with("pull ")
                || action.starts_with("recreate ")
                || action.starts_with("project ")
                || action.starts_with("ensure Cloudflare")
        });
    let ok = materialized && reconcile_ok && publish_ok;
    let message = if !reconcile_ok {
        "deployment reconcile failed; desired deployment was not changed".to_owned()
    } else if !publish_ok {
        "deployment materialized but published route convergence failed".to_owned()
    } else {
        "desired deployment materialized and reconciled".to_owned()
    };
    let typed_events = deploy_apply_typed_events(
        &request.worker,
        Some((&request.image, request.route.as_deref())),
        &reconciliation_steps,
        &execution_typed_events,
        &actions,
        ok,
        &message,
    );
    DeployApplyReport {
        ok,
        worker: request.worker,
        materialized,
        changed,
        actions,
        reconciliation_steps,
        typed_events,
        message: if !reconcile_ok {
            "deployment reconcile failed; desired deployment was not changed".to_owned()
        } else if !publish_ok {
            "deployment materialized but published route convergence failed".to_owned()
        } else {
            "desired deployment materialized and reconciled".to_owned()
        },
    }
}

fn deploy_apply_typed_events(
    worker: &str,
    image_and_route: Option<(&str, Option<&str>)>,
    reconciliation_steps: &[gumgum_core::GraphExecutionStep],
    execution_typed_events: &[gumgum_core::GumgumEvent],
    actions: &[String],
    ok: bool,
    message: &str,
) -> Vec<gumgum_core::GumgumEvent> {
    let mut events = Vec::new();
    if let Some((image, _)) = image_and_route {
        events.push(gumgum_core::GumgumEvent::DeploymentStarted {
            worker: logical_deployment_worker(worker).to_owned(),
            environment: deployment_env(worker),
            image: image.to_owned(),
        });
    }
    if execution_typed_events.is_empty() {
        for step in reconciliation_steps {
            events.push(step.planned_event(None));
        }
    } else {
        events.extend_from_slice(execution_typed_events);
    }
    let action_message = if actions.is_empty() {
        message.to_owned()
    } else {
        actions.join("; ")
    };
    if execution_typed_events.is_empty() {
        for step in reconciliation_steps {
            if ok {
                events.push(step.executed_event(None, action_message.clone()));
            } else {
                events.push(step.failed_event(None, action_message.clone()));
            }
        }
    }
    if let Some((image, route)) = image_and_route {
        if ok {
            events.push(gumgum_core::GumgumEvent::DeploymentSucceeded {
                worker: logical_deployment_worker(worker).to_owned(),
                environment: deployment_env(worker),
                revision: image.rsplit(':').next().map(ToOwned::to_owned),
                route: route.map(ToOwned::to_owned),
            });
        } else {
            events.push(gumgum_core::GumgumEvent::DeploymentFailed {
                worker: logical_deployment_worker(worker).to_owned(),
                environment: deployment_env(worker),
                error: message.to_owned(),
            });
        }
    }
    events
}

fn logical_deployment_worker(worker: &str) -> &str {
    worker.split_once('@').map_or(worker, |(name, _)| name)
}

fn deployment_env(worker: &str) -> Option<String> {
    worker.split_once('@').map(|(_, env)| env.to_owned())
}

fn managed_domain_matches(domains: &[DomainRecord], hostname: &str) -> bool {
    domains
        .iter()
        .any(|domain| hostname == domain.name || hostname.ends_with(&format!(".{}", domain.name)))
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
    request
        .clone()
        .into_desired_deploy()
        .reconciliation_steps(graph_path)
        .await
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

    fn seed_revisions(path: &std::path::Path) -> Vec<gumgum_core::DeploymentRevision> {
        let store = GraphStore::new(path.to_path_buf());
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("api.example.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        let mut second = first.clone();
        second.image = "registry/api:2".to_owned();
        store.materialize_deploy(&second).unwrap();
        let mut third = second.clone();
        third.route = Some("api-v3.example.test".to_owned());
        store.materialize_deploy(&third).unwrap();
        store.deployment_revisions("api", 10).unwrap()
    }

    #[test]
    fn typed_events_ndjson_streams_one_event_per_line() {
        let step = gumgum_core::GraphActionPlanner::ensure_provider_step(
            gumgum_core::ProviderName::new("manual.main").unwrap(),
            gumgum_core::Capability::Manual,
        );
        let events = vec![
            step.planned_event(Some("reconcile-test".to_owned())),
            step.executed_event(
                Some("reconcile-test".to_owned()),
                "configured manual provider manual.main",
            ),
        ];

        let body = typed_events_ndjson(&events);
        let lines = body.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"type\":\"reconcile_step_planned\""));
        assert!(lines[1].contains("\"type\":\"reconcile_step_executed\""));
    }

    #[test]
    fn deploy_stream_filters_events_already_sent_live() {
        assert!(deploy_stream_sends_live(
            &gumgum_core::GumgumEvent::DeploymentStarted {
                worker: "api".to_owned(),
                environment: Some("preview".to_owned()),
                image: "registry/api:rev1".to_owned(),
            },
        ));
        assert!(deploy_stream_sends_live(
            &gumgum_core::GumgumEvent::ReconcileStepExecuted {
                id: None,
                operation_id: None,
                target: "container:api".to_owned(),
                action: "ensure_container".to_owned(),
                message: "recreated container".to_owned(),
                at: None,
            },
        ));
        assert!(!deploy_stream_sends_live(
            &gumgum_core::GumgumEvent::DeploymentSucceeded {
                worker: "api".to_owned(),
                environment: Some("preview".to_owned()),
                revision: Some("rev1".to_owned()),
                route: Some("api.example.test".to_owned()),
            },
        ));
    }

    #[test]
    fn deploy_apply_reports_include_typed_reconcile_events() {
        let step = gumgum_core::GraphActionPlanner::ensure_provider_step(
            gumgum_core::ProviderName::new("manual.main").unwrap(),
            gumgum_core::Capability::Manual,
        );

        let events = deploy_apply_typed_events(
            "api@preview",
            Some(("registry/api:rev1", Some("api.example.test"))),
            &[step],
            &[],
            &["configured manual provider manual.main".to_owned()],
            true,
            "desired deployment materialized and reconciled",
        );

        assert!(matches!(
            events.first(),
            Some(gumgum_core::GumgumEvent::DeploymentStarted {
                worker,
                environment: Some(environment),
                ..
            }) if worker == "api" && environment == "preview"
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            gumgum_core::GumgumEvent::ReconcileStepPlanned { target, .. }
                if target == "provider/manual.main"
        )));
        assert!(matches!(
            events.last(),
            Some(gumgum_core::GumgumEvent::DeploymentSucceeded { route: Some(route), .. })
                if route == "api.example.test"
        ));
    }

    #[tokio::test]
    async fn object_and_binding_mutations_record_activity_and_reconciliation_events() {
        let path = temp_graph_path("object-binding-events");
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(object_report) = daemon_create_object(
            State(state.clone()),
            Json(ObjectRequest {
                capability: gumgum_core::Capability::Queue,
                name: "visit-events".to_owned(),
                namespace: "root".to_owned(),
                root_domain: "leostera.dev".to_owned(),
                password: None,
                preview: false,
            }),
        )
        .await;
        assert!(object_report.ok);
        assert!(object_report.typed_events.iter().any(|event| matches!(
            event,
            gumgum_core::GumgumEvent::ReconcileStepExecuted { target, action, .. }
                if target == "queue/visit-events" && action == "ensure_object"
        )));

        let Json(binding_report) = daemon_create_binding(
            State(state.clone()),
            Json(BindingRequest {
                capability: gumgum_core::Capability::Queue,
                object_name: "visit-events".to_owned(),
                worker: "api".to_owned(),
                binding: "VISIT_EVENTS_QUEUE".to_owned(),
                access: "read-write".to_owned(),
                preview: false,
            }),
        )
        .await;
        assert!(binding_report.ok);
        assert!(binding_report.typed_events.iter().any(|event| matches!(
            event,
            gumgum_core::GumgumEvent::ReconcileStepExecuted { target, action, .. }
                if target == "binding/api/VISIT_EVENTS_QUEUE" && action == "ensure_binding"
        )));

        let events = GraphStore::new(path.clone())
            .list_reconcile_events(20)
            .unwrap();
        assert!(events.iter().any(|event| {
            event.kind == gumgum_core::ControlPlaneEventKind::Mutation
                && event.target == "object/queue/visit-events"
                && event.action == "object.upsert"
        }));
        assert!(events.iter().any(|event| {
            event.kind == gumgum_core::ControlPlaneEventKind::Mutation
                && event.target == "binding/api/VISIT_EVENTS_QUEUE"
                && event.action == "binding.upsert"
        }));
        assert!(events.iter().any(|event| {
            event.kind == gumgum_core::ControlPlaneEventKind::Reconciliation
                && event.target == "queue/visit-events"
                && event.action == "ensure_object"
                && matches!(
                    event.status,
                    gumgum_core::ReconcileEventStatus::Planned
                        | gumgum_core::ReconcileEventStatus::Executed
                )
        }));
        assert!(events.iter().any(|event| {
            event.kind == gumgum_core::ControlPlaneEventKind::Reconciliation
                && event.target == "binding/api/VISIT_EVENTS_QUEUE"
                && event.action == "ensure_binding"
                && event.status == gumgum_core::ReconcileEventStatus::Executed
        }));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn create_previews_explain_object_and_binding_plans_without_mutating() {
        let path = temp_graph_path("create-previews");
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(object_report) = daemon_create_object(
            State(state.clone()),
            Json(ObjectRequest {
                capability: gumgum_core::Capability::Queue,
                name: "visit-events".to_owned(),
                namespace: "root".to_owned(),
                root_domain: "leostera.dev".to_owned(),
                password: None,
                preview: true,
            }),
        )
        .await;
        assert!(object_report.ok);
        assert_eq!(object_report.message, "object create preview");
        assert!(
            object_report
                .provider_actions
                .iter()
                .any(|action| action == "preview only; no objects changed")
        );

        let Json(binding_report) = daemon_create_binding(
            State(state.clone()),
            Json(BindingRequest {
                capability: gumgum_core::Capability::Queue,
                object_name: "visit-events".to_owned(),
                worker: "api".to_owned(),
                binding: "VISIT_EVENTS_QUEUE".to_owned(),
                access: "read-write".to_owned(),
                preview: true,
            }),
        )
        .await;
        assert!(binding_report.ok);
        assert_eq!(binding_report.message, "binding create preview");
        assert!(
            binding_report
                .binding_actions
                .iter()
                .any(|action| action == "preview only; no bindings changed")
        );

        let store = GraphStore::new(path.clone());
        let graph = store.load_desired_graph().unwrap();
        assert!(!graph.nodes.iter().any(|node| matches!(
            node,
            gumgum_core::DesiredGraphNode::Object { name, .. }
                if name.as_str() == "visit-events"
        )));
        assert!(!graph.nodes.iter().any(|node| matches!(
            node,
            gumgum_core::DesiredGraphNode::Binding { worker, name, .. }
                if worker.as_str() == "api" && name.as_str() == "VISIT_EVENTS_QUEUE"
        )));
        assert!(store.list_reconcile_events(20).unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn delete_previews_explain_object_and_binding_plans_without_mutating() {
        let path = temp_graph_path("delete-previews");
        let store = GraphStore::new(path.clone());
        store
            .materialize_object(&gumgum_core::GlobalObject {
                capability: gumgum_core::Capability::Queue,
                name: "visit-events".to_owned(),
                namespace: "root".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&gumgum_core::WorkerBinding {
                capability: gumgum_core::Capability::Queue,
                object_name: "visit-events".to_owned(),
                worker: "api".to_owned(),
                binding: "VISIT_EVENTS_QUEUE".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        let before_events = store.list_reconcile_events(20).unwrap().len();
        let state = DaemonState {
            graph_path: Arc::new(path.clone()),
        };

        let Json(binding_report) = daemon_delete_binding(
            State(state.clone()),
            Json(BindingDeleteRequest {
                capability: gumgum_core::Capability::Queue,
                object_name: "visit-events".to_owned(),
                worker: "api".to_owned(),
                binding: "VISIT_EVENTS_QUEUE".to_owned(),
                preview: true,
            }),
        )
        .await;
        assert!(binding_report.ok);
        assert_eq!(binding_report.message, "binding delete preview");
        assert!(!binding_report.reconciliation_steps.is_empty());
        assert_eq!(
            binding_report.binding_actions,
            vec!["preview only; no bindings changed"]
        );

        let Json(object_report) = daemon_delete_object(
            State(state),
            Json(ObjectDeleteRequest {
                capability: gumgum_core::Capability::Queue,
                name: "visit-events".to_owned(),
                namespace: "root".to_owned(),
                root_domain: "leostera.dev".to_owned(),
                preview: true,
            }),
        )
        .await;
        assert!(!object_report.ok);
        assert_eq!(
            object_report.message,
            "object has active bindings; unbind it before deleting"
        );
        assert!(object_report.reconciliation_steps.is_empty());
        assert_eq!(
            object_report.provider_actions,
            vec!["still bound to worker api as VISIT_EVENTS_QUEUE"]
        );
        assert_eq!(
            GraphStore::new(path.clone())
                .list_reconcile_events(20)
                .unwrap()
                .len(),
            before_events
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn daemon_events_lists_newest_reconciliation_events() {
        let path = temp_graph_path("events-list");
        let store = GraphStore::new(path.clone());
        store
            .record_reconcile_event(&gumgum_core::NewReconcileEvent {
                kind: gumgum_core::ControlPlaneEventKind::Reconciliation,
                operation_id: None,
                status: gumgum_core::ReconcileEventStatus::Planned,
                target: "provider/manual.main".to_owned(),
                action: "ensure_provider".to_owned(),
                message: "plan provider".to_owned(),
            })
            .unwrap();
        store
            .record_reconcile_event(&gumgum_core::NewReconcileEvent {
                kind: gumgum_core::ControlPlaneEventKind::Reconciliation,
                operation_id: None,
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
    fn daemon_version_report_advertises_safe_delete_rollback_and_bucket_capabilities() {
        let report = daemon_version_report();

        assert!(report.ok);
        assert!(
            report
                .capabilities
                .contains(&"gumgum:rollback:revisions".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:rollback:revision_id".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:rollback:revision_delete".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:objects:create_preview".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:bindings:create_preview".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:bindings:delete".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:objects:delete".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:deployments:delete".to_owned())
        );
        assert!(
            report
                .capabilities
                .contains(&"gumgum:buckets:objects".to_owned())
        );
    }

    #[test]
    fn missing_provider_credentials_report_blocks_object_creation() {
        let report = missing_provider_credentials_report(
            "blob".to_owned(),
            "uploads".to_owned(),
            "uploads.bucket.example.test".to_owned(),
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
                route: Some("api.example.test".to_owned()),
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
            route: Some("api.example.test".to_owned()),
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
        assert_eq!(report.route, selected.deploy.route);
        assert!(
            report
                .actions
                .iter()
                .any(|action| action == "preview only; no containers changed")
        );
        assert!(report.actions.iter().any(|action| action
            == "warning: rollback would change route from api-v3.example.test to api.example.test"));
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
