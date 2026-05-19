use anyhow::Result;
use axum::{Json, Router, routing::get};
use gumgum_api::not_configured_status;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .with_target(false)
        .init();

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v0/status", get(status));
    let addr = SocketAddr::from(([0, 0, 0, 0], 7777));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "gumgumd listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "gumgumd" }))
}

async fn status() -> Json<gumgum_core::StatusReport> {
    Json(not_configured_status())
}
