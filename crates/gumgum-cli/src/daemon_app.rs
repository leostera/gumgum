use crate::{ensure_local_platform, gumgum_root};
use axum::{
    Json, Router,
    routing::{get, post},
};
use gumgum_api::not_configured_status;
use gumgum_core::{ErrorCode, GraphStore, GumgumError, Subsystem};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

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
            .route("/v0/deploy", post(crate::daemon_deploy))
            .route("/v0/rollback", post(crate::daemon_rollback))
            .route("/v0/objects", post(crate::daemon_create_object))
            .route("/v0/bindings", post(crate::daemon_create_binding))
            .route("/v0/graph", get(crate::daemon_graph))
            .route("/v0/graph/affected", get(crate::daemon_graph_affected))
            .route("/v0/logs/{container}", get(crate::daemon_logs))
            .with_state(state)
    }
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "gumgumd" }))
}

async fn status() -> Json<gumgum_core::StatusReport> {
    Json(not_configured_status())
}
