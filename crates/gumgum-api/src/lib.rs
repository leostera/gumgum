use gumgum_core::Capability;
pub use gumgum_core::{DeploymentRevision, GraphEdge, GraphNode, ProviderStatus, ServerRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SetupPlan {
    pub ok: bool,
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub root_domain: String,
    pub test_domain: String,
    pub actions: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetupReport {
    pub ok: bool,
    pub name: String,
    pub host: String,
    pub root_domain: String,
    pub test_domain: String,
    pub service: String,
    pub health_url: String,
    pub actions: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ServerListReport {
    pub ok: bool,
    pub servers: Vec<ServerRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PingReport {
    pub ok: bool,
    pub host: String,
    pub health_url: String,
    pub service_active: Option<bool>,
    pub health: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployRequest {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: String,
    pub port: u16,
    pub health: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployApplyReport {
    pub ok: bool,
    pub worker: String,
    pub materialized: bool,
    pub changed: bool,
    pub actions: Vec<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogsReport {
    pub ok: bool,
    pub container: String,
    pub tail: u32,
    pub logs: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphReport {
    pub ok: bool,
    pub format: String,
    pub graph: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<GraphNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObjectRequest {
    pub capability: Capability,
    pub name: String,
    pub namespace: String,
    pub root_domain: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObjectReport {
    pub ok: bool,
    pub kind: String,
    pub name: String,
    pub dns: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_actions: Vec<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BindingRequest {
    pub capability: Capability,
    pub object_name: String,
    pub worker: String,
    pub binding: String,
    pub access: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BindingReport {
    pub ok: bool,
    pub object: String,
    pub worker: String,
    pub binding: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderStatusReport {
    pub ok: bool,
    pub providers: Vec<ProviderStatus>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AffectedReport {
    pub ok: bool,
    pub target: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonVersionReport {
    pub ok: bool,
    pub version: String,
    pub git_sha: String,
    pub target: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RollbackRequest {
    pub worker: String,
    #[serde(default)]
    pub preview: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RollbackReport {
    pub ok: bool,
    pub worker: String,
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    pub actions: Vec<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeploymentRevisionsReport {
    pub ok: bool,
    pub worker: String,
    pub revisions: Vec<DeploymentRevision>,
    pub message: String,
}
