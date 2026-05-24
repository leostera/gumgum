use crate::{CoreAction, DaemonStatus, SetupStep, StatusReport};

pub fn setup_actions(local: bool) -> Vec<CoreAction> {
    let steps = if local {
        vec![
            SetupStep::CreateLocalDirectories,
            SetupStep::InstallRunningBinary,
            SetupStep::WriteUserSystemdService,
            SetupStep::EnableRestartDaemon,
            SetupStep::CheckLocalHealth,
        ]
    } else {
        vec![
            SetupStep::SshIntoHost,
            SetupStep::RunRemoteInstaller,
            SetupStep::RunRemoteSetup,
            SetupStep::ExitSsh,
            SetupStep::SaveServerLocally,
            SetupStep::CheckRemoteHealth,
        ]
    };
    steps
        .into_iter()
        .map(|step| CoreAction::SetupStep { step })
        .collect()
}

pub fn not_configured_status() -> StatusReport {
    StatusReport {
        ok: true,
        configured: false,
        daemon: DaemonStatus::NotConfigured,
        message: "GumGum.dev is not configured on this machine yet".to_owned(),
    }
}
