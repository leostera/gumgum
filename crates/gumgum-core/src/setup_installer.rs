use crate::{
    ErrorCode, ErrorKind, GumgumError, IngressMode, Subsystem, run_setup_command_streaming,
    sanitize_name,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tokio::process::Command as TokioCommand;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetupTarget {
    pub name: String,
    pub host: String,
    pub user: Option<String>,
    pub root_domain: String,
    pub test_domain: String,
    pub local: bool,
    pub ingress: IngressMode,
}

impl SetupTarget {
    pub fn ssh_target(&self) -> String {
        ssh_target(self.user.as_deref(), &self.host)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SetupOptions {
    pub host: Option<String>,
    pub name: Option<String>,
    pub user: Option<String>,
    pub root_domain: Option<String>,
    pub test_domain: Option<String>,
    pub ingress: Option<IngressMode>,
}

pub struct GumgumInstaller;

impl GumgumInstaller {
    pub async fn resolve_target(options: SetupOptions) -> crate::Result<SetupTarget> {
        let host = options.host.unwrap_or_else(|| "127.0.0.1".to_owned());
        let local = matches!(host.as_str(), "127.0.0.1" | "localhost" | "0.0.0.0");
        let target = ssh_target(options.user.as_deref(), &host);
        let name = match options.name {
            Some(name) => name,
            None if local => local_hostname().await?,
            None => remote_hostname(&target)
                .await
                .unwrap_or_else(|_| sanitize_name(&host)),
        };
        let root_domain = options.root_domain.unwrap_or_else(|| format!("{name}.dev"));
        let test_domain = options.test_domain.unwrap_or_default();
        Ok(SetupTarget {
            name,
            host,
            user: options.user,
            root_domain,
            test_domain,
            local,
            ingress: options.ingress.unwrap_or_default(),
        })
    }

    pub async fn install_local_user_service(quiet: bool) -> crate::Result<()> {
        let gumgum = std::env::current_exe()
            .map_err(|source| setup_io_error(ErrorKind::SetupBinaryLocateFailed, source))?;
        let home = std::env::var("HOME")
            .map_err(|source| setup_io_error(ErrorKind::HomeReadFailed, source))?;
        fs::create_dir_all(format!("{home}/.gumgum/daemon")).map_err(|source| {
            setup_io_error(ErrorKind::SetupDaemonDirectoryCreateFailed, source)
        })?;
        fs::create_dir_all(format!("{home}/.gumgum/bin"))
            .map_err(|source| setup_io_error(ErrorKind::SetupBinDirectoryCreateFailed, source))?;
        let installed_gumgum = PathBuf::from(format!("{home}/.gumgum/bin/gumgum"));
        if gumgum != installed_gumgum {
            fs::copy(&gumgum, &installed_gumgum).map_err(|source| {
                setup_io_error(ErrorKind::SetupLocalDaemonInstallFailed, source)
            })?;
        }
        run_setup_command_streaming(
            TokioCommand::new("chmod")
                .arg("0755")
                .arg(&installed_gumgum),
            quiet,
        )
        .await?;
        fs::create_dir_all(format!("{home}/.config/systemd/user")).map_err(|source| {
            setup_io_error(ErrorKind::SetupUserSystemdDirectoryCreateFailed, source)
        })?;
        fs::write(
            format!("{home}/.gumgum/daemon/gumgumd.service"),
            user_systemd_service(),
        )
        .map_err(|source| setup_io_error(ErrorKind::SetupLocalUserServiceWriteFailed, source))?;
        run_setup_command_streaming(
            TokioCommand::new("ln")
                .arg("-sf")
                .arg(format!("{home}/.gumgum/daemon/gumgumd.service"))
                .arg(format!("{home}/.config/systemd/user/gumgumd.service")),
            quiet,
        )
        .await?;
        run_setup_command_streaming(
            TokioCommand::new("systemctl")
                .arg("--user")
                .arg("daemon-reload"),
            quiet,
        )
        .await?;
        run_setup_command_streaming(
            TokioCommand::new("systemctl")
                .arg("--user")
                .arg("enable")
                .arg("--now")
                .arg("gumgumd"),
            quiet,
        )
        .await?;
        run_setup_command_streaming(
            TokioCommand::new("systemctl")
                .arg("--user")
                .arg("restart")
                .arg("gumgumd"),
            quiet,
        )
        .await
    }

    pub async fn configure_host_dns(test_domain: &str, quiet: bool) -> crate::Result<()> {
        let domain = shell_escape_plain(test_domain);
        let script = format!(
            "set -e; ip=$(hostname -I 2>/dev/null | awk '{{print $1}}'); [ -n \"$ip\" ] || ip=127.0.0.1; if [ -w $HOME/.gumgum/dnsmasq/dnsmasq.conf ]; then if ! grep -q '^address=/{domain}/' $HOME/.gumgum/dnsmasq/dnsmasq.conf; then printf '\n# GumGum.dev test domain\naddress=/{domain}/%s\n' \"$ip\" >> $HOME/.gumgum/dnsmasq/dnsmasq.conf; fi; if docker inspect gumgum-dnsmasq >/dev/null 2>&1; then docker restart gumgum-dnsmasq >/dev/null; fi; fi; if docker inspect dnsmasq >/dev/null 2>&1 && [ -w /apps/fleet/gateway/dnsmasq/dnsmasq.conf ]; then if ! grep -q '^address=/{domain}/' /apps/fleet/gateway/dnsmasq/dnsmasq.conf; then printf '\n# GumGum.dev test domain\naddress=/{domain}/%s\n' \"$ip\" >> /apps/fleet/gateway/dnsmasq/dnsmasq.conf; fi; docker restart dnsmasq >/dev/null; fi"
        );
        run_setup_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
    }

    pub async fn configure_client_resolver(
        test_domain: &str,
        host: &str,
        quiet: bool,
    ) -> crate::Result<()> {
        match std::env::consts::OS {
            "macos" => {
                let script = format!(
                    "set -e; if [ ! -t 0 ] && ! sudo -n true 2>/dev/null; then exit 0; fi; sudo mkdir -p /etc/resolver; printf 'nameserver {host}\n' | sudo tee /etc/resolver/{domain} >/dev/null; sudo dscacheutil -flushcache",
                    host = shell_escape_plain(host),
                    domain = shell_escape_plain(test_domain)
                );
                run_setup_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet)
                    .await
            }
            _ => Ok(()),
        }
    }

    pub async fn run_remote_setup(
        target: &str,
        setup: &SetupTarget,
        quiet: bool,
    ) -> crate::Result<()> {
        let remote_setup = remote_setup_command(setup, quiet);
        let script = format!(
            "set -e; primary=https://get.gumgum.dev; fallback=https://get-gumgum-dev.abstractmachines.workers.dev; tmp=$(mktemp); trap 'rm -f $tmp' EXIT; if command -v curl >/dev/null 2>&1; then if curl -fsSL -o $tmp $primary; then GUMGUM_NO_PATH=1 sh $tmp; else curl -fsSL -o $tmp $fallback; GUMGUM_BASE_URL=$fallback GUMGUM_NO_PATH=1 sh $tmp; fi; elif command -v wget >/dev/null 2>&1; then if wget -q -O $tmp $primary; then GUMGUM_NO_PATH=1 sh $tmp; else wget -q -O $tmp $fallback; GUMGUM_BASE_URL=$fallback GUMGUM_NO_PATH=1 sh $tmp; fi; else exit 1; fi; {remote_setup}"
        );
        run_setup_command_streaming(TokioCommand::new("ssh").arg(target).arg(script), quiet).await
    }
}

fn remote_setup_command(setup: &SetupTarget, quiet: bool) -> String {
    format!(
        "~/.gumgum/bin/gumgum setup 127.0.0.1 --name {} --domain {}{}",
        shell_quote(&setup.name),
        shell_quote(&setup.root_domain),
        if quiet { " --json" } else { "" }
    )
}

async fn local_hostname() -> crate::Result<String> {
    let output = TokioCommand::new("hostname")
        .output()
        .await
        .map_err(|source| setup_io_error(ErrorKind::SetupLocalHostnameReadFailed, source))?;
    if !output.status.success() {
        return Ok("localhost".to_owned());
    }
    Ok(sanitize_name(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

async fn remote_hostname(target: &str) -> crate::Result<String> {
    let output = TokioCommand::new("ssh")
        .arg(target)
        .arg("hostname")
        .output()
        .await
        .map_err(|source| setup_io_error(ErrorKind::SetupRemoteHostnameReadFailed, source))?;
    if !output.status.success() {
        return Err(GumgumError::structured_kind(
            Subsystem::Setup,
            ErrorCode::Io,
            ErrorKind::SetupRemoteHostnameCommandFailed,
        )
        .build());
    }
    Ok(sanitize_name(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn setup_io_error(kind: ErrorKind, source: impl std::fmt::Display) -> GumgumError {
    GumgumError::structured_kind(Subsystem::Setup, ErrorCode::Io, kind)
        .likely_cause(source.to_string())
        .build()
}

fn ssh_target(user: Option<&str>, host: &str) -> String {
    match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_escape_plain(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        .collect()
}

fn user_systemd_service() -> &'static str {
    r#"[Unit]
Description=GumGum.dev daemon
After=network-online.target

[Service]
Type=simple
ExecStart=%h/.gumgum/bin/gumgum daemon
Restart=always
RestartSec=2
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#
}
