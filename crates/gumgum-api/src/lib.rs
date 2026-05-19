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
    ) -> Self {
        Self {
            ok: true,
            name,
            host,
            user,
            root_domain,
            test_domain,
            actions: setup_actions(),
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

pub fn setup_actions() -> Vec<String> {
    vec![
        "ssh into host".to_owned(),
        "create GumGum.dev state directory".to_owned(),
        "install gumgumd binary".to_owned(),
        "write gumgumd systemd service".to_owned(),
        "enable and restart gumgumd".to_owned(),
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

pub fn not_configured_status() -> StatusReport {
    StatusReport {
        ok: true,
        configured: false,
        daemon: DaemonStatus::NotConfigured,
        message: "GumGum.dev is not configured on this machine yet".to_owned(),
    }
}
