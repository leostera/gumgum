use gumgum_core::{DaemonStatus, StatusReport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct SetupPlan {
    pub ok: bool,
    pub host: String,
    pub user: Option<String>,
    pub root_domain: String,
    pub test_domain: String,
    pub actions: Vec<String>,
}

impl SetupPlan {
    pub fn dry_run(
        host: String,
        user: Option<String>,
        root_domain: String,
        test_domain: String,
    ) -> Self {
        Self {
            ok: true,
            host,
            user,
            root_domain,
            test_domain,
            actions: vec![
                "ssh into host".to_owned(),
                "detect os and architecture".to_owned(),
                "install gumgumd".to_owned(),
                "initialize graph store".to_owned(),
                "configure provider defaults".to_owned(),
            ],
        }
    }
}

pub fn not_configured_status() -> StatusReport {
    StatusReport {
        ok: true,
        configured: false,
        daemon: DaemonStatus::NotConfigured,
        message: "GumGum.dev is not configured on this machine yet".to_owned(),
    }
}
