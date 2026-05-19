use gumgum_core::{DaemonStatus, StatusReport};
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

impl SetupPlan {
    pub fn dry_run(
        name: String,
        host: String,
        user: Option<String>,
        root_domain: String,
        test_domain: String,
        local: bool,
    ) -> Self {
        Self {
            ok: true,
            name,
            host,
            user,
            root_domain,
            test_domain,
            actions: setup_actions(local),
        }
    }
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

pub fn setup_actions(local: bool) -> Vec<String> {
    if local {
        return vec![
            "create ~/.gumgum/bin and ~/.gumgum/daemon".to_owned(),
            "install running gumgum binary into ~/.gumgum/bin".to_owned(),
            "write gumgumd user-systemd service".to_owned(),
            "enable and restart gumgumd".to_owned(),
            "check http://127.0.0.1:7777/healthz".to_owned(),
        ];
    }

    vec![
        "ssh into host".to_owned(),
        "run curl -fsSL https://get.gumgum.dev | sh".to_owned(),
        "run ~/.gumgum/bin/gumgum setup on the host".to_owned(),
        "exit ssh".to_owned(),
        "save server locally".to_owned(),
        "configure local resolver for test domain".to_owned(),
        "check http://<host>:7777/healthz".to_owned(),
    ]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerRecord {
    pub name: String,
    pub host: String,
    pub root_domain: String,
    pub test_domain: String,
    pub health_url: String,
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
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObjectRequest {
    pub kind: String,
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
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BindingRequest {
    pub object_kind: String,
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
pub struct AffectedReport {
    pub ok: bool,
    pub target: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub message: String,
}

pub fn not_configured_status() -> StatusReport {
    StatusReport {
        ok: true,
        configured: false,
        daemon: DaemonStatus::NotConfigured,
        message: "GumGum.dev is not configured on this machine yet".to_owned(),
    }
}
