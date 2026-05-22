use crate::{DaemonStatus, StatusReport};

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
        "check http://<host>:7777/healthz".to_owned(),
    ]
}

pub fn not_configured_status() -> StatusReport {
    StatusReport {
        ok: true,
        configured: false,
        daemon: DaemonStatus::NotConfigured,
        message: "GumGum.dev is not configured on this machine yet".to_owned(),
    }
}
