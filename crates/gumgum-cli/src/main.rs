use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use gumgum_api::{
    PingReport, ServerListReport, ServerRecord, SetupPlan, SetupReport, not_configured_status,
    setup_actions,
};
use gumgum_core::{DoctorCheck, DoctorReport, ErrorCode, GumgumError, Subsystem};
use gumgum_manifest::validate_path;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command as TokioCommand;

#[derive(Debug, Parser)]
#[command(name = "gumgum")]
#[command(about = "GumGum.dev local cloud control plane")]
struct Cli {
    #[arg(long, global = true, help = "Emit stable JSON output")]
    json: bool,
    #[arg(long, global = true, help = "Plan without mutating state")]
    dry_run: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Status(StatusArgs),
    Ping(PingArgs),
    Doctor,
    Setup(SetupArgs),
    Server(ServerCommand),
    Schema(SchemaCommand),
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    user: Option<String>,
}

#[derive(Debug, Args)]
struct PingArgs {
    host: String,
    #[arg(long)]
    user: Option<String>,
}

#[derive(Debug, Args)]
struct SetupArgs {
    host: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    user: Option<String>,
    #[arg(long)]
    root_domain: Option<String>,
    #[arg(long)]
    test_domain: Option<String>,
}

#[derive(Debug, Args)]
struct ServerCommand {
    #[command(subcommand)]
    command: ServerSubcommand,
}

#[derive(Debug, Subcommand)]
enum ServerSubcommand {
    List,
}

#[derive(Debug, Args)]
struct SchemaCommand {
    #[command(subcommand)]
    command: SchemaSubcommand,
}

#[derive(Debug, Subcommand)]
enum SchemaSubcommand {
    Validate { path: Option<PathBuf> },
    Explain,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .with_target(false)
        .init();

    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        print_error(err);
        std::process::exit(1);
    }
    Ok(())
}

async fn run(cli: Cli) -> gumgum_core::Result<()> {
    match cli.command {
        Command::Status(args) => {
            if let Some(host) = args.host {
                let report = ping_host(&host).await?;
                print_value(cli.json, &report)
            } else if let Some(server) = load_default_server()? {
                let report = ping_host(&server.host).await?;
                print_value(cli.json, &report)
            } else {
                print_value(cli.json, &not_configured_status())
            }
        }
        Command::Ping(args) => {
            let report = ping_host(&args.host).await?;
            print_value(cli.json, &report)
        }
        Command::Doctor => {
            let report = DoctorReport {
                ok: true,
                checks: vec![
                    DoctorCheck {
                        name: "cli".to_owned(),
                        ok: true,
                        message: "gumgum CLI is installed".to_owned(),
                    },
                    DoctorCheck {
                        name: "daemon".to_owned(),
                        ok: true,
                        message: "daemon check skipped until setup is implemented".to_owned(),
                    },
                ],
            };
            print_value(cli.json, &report)
        }
        Command::Setup(args) => {
            let resolved = resolve_setup(args).await?;
            if cli.dry_run {
                let plan = SetupPlan::dry_run(
                    resolved.name,
                    resolved.host,
                    resolved.user,
                    resolved.root_domain,
                    resolved.test_domain,
                );
                print_value(cli.json, &plan)
            } else {
                let report = install_gumgumd(resolved, cli.json).await?;
                print_value(cli.json, &report)
            }
        }
        Command::Server(server) => match server.command {
            ServerSubcommand::List => {
                let report = ServerListReport {
                    ok: true,
                    servers: load_servers()?,
                };
                print_value(cli.json, &report)
            }
        },
        Command::Schema(schema) => match schema.command {
            SchemaSubcommand::Validate { path } => {
                let path = path.unwrap_or_else(|| PathBuf::from("gumgum.toml"));
                let report = validate_path(&path)?;
                print_value(cli.json, &report)
            }
            SchemaSubcommand::Explain => {
                let explanation = SchemaExplanation {
                    ok: true,
                    schemas: vec!["workspace", "worker"],
                    message: "v0 supports [workspace] and [worker] manifests".to_owned(),
                };
                print_value(cli.json, &explanation)
            }
        },
    }
    Ok(())
}

#[derive(Debug)]
struct ResolvedSetup {
    name: String,
    host: String,
    user: Option<String>,
    root_domain: String,
    test_domain: String,
    local: bool,
}

async fn resolve_setup(args: SetupArgs) -> gumgum_core::Result<ResolvedSetup> {
    let local = args.host.is_none();
    let host = args.host.unwrap_or_else(|| "127.0.0.1".to_owned());
    let target = ssh_target(args.user.as_deref(), &host);
    let name = match args.name {
        Some(name) => name,
        None if local => local_hostname().await?,
        None => remote_hostname(&target)
            .await
            .unwrap_or_else(|_| sanitize_name(&host)),
    };
    let root_domain = args.root_domain.unwrap_or_else(|| format!("{name}.dev"));
    let test_domain = args
        .test_domain
        .unwrap_or_else(|| derive_test_domain(&root_domain));
    Ok(ResolvedSetup {
        name,
        host,
        user: args.user,
        root_domain,
        test_domain,
        local,
    })
}

async fn local_hostname() -> gumgum_core::Result<String> {
    let output = TokioCommand::new("hostname")
        .output()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to read local hostname",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if !output.status.success() {
        return Ok("localhost".to_owned());
    }
    Ok(sanitize_name(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

async fn remote_hostname(target: &str) -> gumgum_core::Result<String> {
    let output = TokioCommand::new("ssh")
        .arg(target)
        .arg("hostname")
        .output()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to read remote hostname",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if !output.status.success() {
        return Err(GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "remote hostname failed",
        )
        .build());
    }
    Ok(sanitize_name(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn sanitize_name(value: &str) -> String {
    let name: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    name.trim_matches('-').to_owned()
}

async fn ping_host(host: &str) -> gumgum_core::Result<PingReport> {
    let health_url = format!("http://{host}:7777/healthz");
    let health: serde_json::Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|source| {
            GumgumError::structured(Subsystem::Api, ErrorCode::Io, "failed to build HTTP client")
                .likely_cause(source.to_string())
                .build()
        })?
        .get(&health_url)
        .send()
        .await
        .map_err(|source| {
            GumgumError::structured(Subsystem::Api, ErrorCode::Io, "failed to reach gumgumd")
                .likely_cause(source.to_string())
                .next_command(format!("gumgum setup {host} --root-domain <domain>"))
                .build()
        })?
        .error_for_status()
        .map_err(|source| {
            GumgumError::structured(Subsystem::Api, ErrorCode::Io, "gumgumd returned an error")
                .likely_cause(source.to_string())
                .build()
        })?
        .json()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "gumgumd returned invalid JSON",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    let ok = health
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(PingReport {
        ok,
        host: host.to_owned(),
        health_url,
        service_active: Some(ok),
        health,
    })
}

async fn install_gumgumd(setup: ResolvedSetup, quiet: bool) -> gumgum_core::Result<SetupReport> {
    progress(quiet, "resolving setup target");
    if setup.local {
        progress(quiet, "building gumgumd locally");
        build_local_gumgumd().await?;
        progress(quiet, "installing local user service into ~/.gumgum/daemon");
        install_local_user_service().await?;
    } else {
        let target = ssh_target(setup.user.as_deref(), &setup.host);
        progress(quiet, format!("building gumgumd on {target}"));
        let remote_gumgumd = build_remote_gumgumd(&target, quiet).await?;
        progress(quiet, "detecting install mode");
        let install_mode = detect_install_mode(&target).await?;
        progress(quiet, "installing gumgumd service");
        match install_mode {
            InstallMode::System => install_system_service(&target, &remote_gumgumd).await?,
            InstallMode::User => install_user_service(&target, &remote_gumgumd).await?,
        }
    }
    progress(quiet, "checking gumgumd health");
    ping_host(&setup.host).await?;
    let health_url = format!("http://{}:7777/healthz", setup.host);
    save_server(ServerRecord {
        name: setup.name.clone(),
        host: setup.host.clone(),
        root_domain: setup.root_domain.clone(),
        test_domain: setup.test_domain.clone(),
        health_url: health_url.clone(),
    })?;
    Ok(SetupReport {
        ok: true,
        name: setup.name,
        host: setup.host,
        root_domain: setup.root_domain,
        test_domain: setup.test_domain,
        service: "gumgumd".to_owned(),
        health_url,
        actions: setup_actions(),
    })
}

#[derive(Clone, Copy, Debug)]
enum InstallMode {
    System,
    User,
}

async fn build_local_gumgumd() -> gumgum_core::Result<()> {
    let root = workspace_root().ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not locate workspace root",
        )
        .likely_cause("gumgum setup currently expects to run from a Cargo workspace checkout")
        .build()
    })?;
    run_command(
        TokioCommand::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("gumgumd")
            .current_dir(root),
    )
    .await
}

async fn install_local_user_service() -> gumgum_core::Result<()> {
    let root = workspace_root().ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not locate workspace root",
        )
        .build()
    })?;
    let gumgumd = root.join("target/debug/gumgumd");
    let home = std::env::var("HOME").map_err(|source| {
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "could not read HOME")
            .likely_cause(source.to_string())
            .build()
    })?;
    fs::create_dir_all(format!("{home}/.gumgum/daemon")).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not create ~/.gumgum/daemon",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    fs::copy(&gumgumd, format!("{home}/.gumgum/daemon/gumgumd")).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not install local gumgumd",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    run_command(
        TokioCommand::new("chmod")
            .arg("0755")
            .arg(format!("{home}/.gumgum/daemon/gumgumd")),
    )
    .await?;
    fs::create_dir_all(format!("{home}/.config/systemd/user")).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not create user systemd dir",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    fs::write(
        format!("{home}/.gumgum/daemon/gumgumd.service"),
        user_systemd_service(),
    )
    .map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not write local user service",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    run_command(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("daemon-reload"),
    )
    .await?;
    run_command(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("enable")
            .arg("--now")
            .arg("gumgumd"),
    )
    .await?;
    run_command(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("restart")
            .arg("gumgumd"),
    )
    .await
}

async fn build_remote_gumgumd(target: &str, quiet: bool) -> gumgum_core::Result<String> {
    let root = workspace_root().ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not locate workspace root",
        )
        .likely_cause("gumgum setup currently expects to run from a Cargo workspace checkout")
        .build()
    })?;
    progress(quiet, "syncing source to remote host");
    run_command(
        TokioCommand::new("rsync")
            .arg("-az")
            .arg("--delete")
            .arg("--exclude")
            .arg("target")
            .arg("--exclude")
            .arg(".git")
            .arg(format!("{}/", root.display()))
            .arg(format!("{target}:/tmp/gumgum-src/")),
    )
    .await?;
    progress(quiet, "compiling gumgumd on remote host");
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg("cd /tmp/gumgum-src && cargo build -q -p gumgumd"),
    )
    .await?;
    Ok("/tmp/gumgum-src/target/debug/gumgumd".to_owned())
}

fn workspace_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let target_dir = exe.parent()?.parent()?;
    target_dir.parent().map(Path::to_path_buf)
}

async fn detect_install_mode(target: &str) -> gumgum_core::Result<InstallMode> {
    let output = TokioCommand::new("ssh")
        .arg(target)
        .arg("id -u")
        .output()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to detect remote user",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "0" {
        Ok(InstallMode::System)
    } else {
        Ok(InstallMode::User)
    }
}

async fn install_system_service(target: &str, remote_gumgumd: &str) -> gumgum_core::Result<()> {
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg(format!("install -d -m 0755 /var/lib/gumgum /usr/local/bin && install -m 0755 {remote_gumgumd} /usr/local/bin/gumgumd")),
    )
    .await?;
    let service = systemd_service();
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg(format!("cat > /etc/systemd/system/gumgumd.service <<'EOF'\n{service}\nEOF\nsystemctl daemon-reload\nsystemctl enable --now gumgumd\nsystemctl restart gumgumd")),
    )
    .await
}

async fn install_user_service(target: &str, remote_gumgumd: &str) -> gumgum_core::Result<()> {
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg(format!("mkdir -p ~/.gumgum/daemon ~/.config/systemd/user && install -m 0755 {remote_gumgumd} ~/.gumgum/daemon/gumgumd")),
    )
    .await?;
    let service = user_systemd_service();
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg(format!("cat > ~/.gumgum/daemon/gumgumd.service <<'EOF'\n{service}\nEOF\nln -sf ~/.gumgum/daemon/gumgumd.service ~/.config/systemd/user/gumgumd.service\nsystemctl --user daemon-reload\nsystemctl --user enable --now gumgumd\nsystemctl --user restart gumgumd")),
    )
    .await
}

fn gumgum_root() -> gumgum_core::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not read HOME")
            .likely_cause(source.to_string())
            .build()
    })?;
    Ok(PathBuf::from(home).join(".gumgum"))
}

fn config_path() -> gumgum_core::Result<PathBuf> {
    Ok(gumgum_root()?.join("servers.json"))
}

fn load_servers() -> gumgum_core::Result<Vec<ServerRecord>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not read server list",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    serde_json::from_str(&raw).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not parse server list",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

fn load_default_server() -> gumgum_core::Result<Option<ServerRecord>> {
    Ok(load_servers()?.into_iter().next())
}

fn save_server(server: ServerRecord) -> gumgum_core::Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not create config directory",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    }
    let mut servers = load_servers()?;
    servers.retain(|existing| existing.host != server.host);
    servers.insert(0, server);
    let raw = serde_json::to_string_pretty(&servers).expect("serialize servers");
    fs::write(&path, raw).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not write server list",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

async fn run_command(cmd: &mut TokioCommand) -> gumgum_core::Result<()> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "failed to run setup command",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "setup command failed")
            .likely_cause(if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            })
            .next_command("gumgum setup <host> --root-domain <domain> --dry-run")
            .build(),
    )
}

fn ssh_target(user: Option<&str>, host: &str) -> String {
    match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    }
}

fn systemd_service() -> &'static str {
    r#"[Unit]
Description=GumGum.dev daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/gumgumd
Restart=on-failure
RestartSec=2
User=root

[Install]
WantedBy=multi-user.target"#
}

fn user_systemd_service() -> &'static str {
    r#"[Unit]
Description=GumGum.dev daemon
After=default.target

[Service]
Type=simple
ExecStart=%h/.gumgum/daemon/gumgumd
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target"#
}

fn progress(quiet: bool, message: impl AsRef<str>) {
    if !quiet {
        eprintln!("→ {}", message.as_ref());
    }
}

fn print_value<T: Serialize>(json: bool, value: &T) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("serialize json")
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("serialize json")
        );
    }
}

fn print_error(err: GumgumError) {
    println!(
        "{}",
        serde_json::to_string_pretty(&err.to_report()).expect("serialize error")
    );
}

fn derive_test_domain(root_domain: &str) -> String {
    let root = root_domain.trim_end_matches('.');
    match root.rsplit_once('.') {
        Some((name, _)) => format!("{name}.test"),
        None => format!("{root}.test"),
    }
}

#[derive(Debug, Serialize)]
struct SchemaExplanation {
    ok: bool,
    schemas: Vec<&'static str>,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::derive_test_domain;

    #[test]
    fn derives_test_domain_from_root_domain() {
        assert_eq!(derive_test_domain("leostera.dev"), "leostera.test");
    }

    #[test]
    fn formats_ssh_target() {
        assert_eq!(super::ssh_target(None, "192.168.0.3"), "192.168.0.3");
        assert_eq!(
            super::ssh_target(Some("root"), "192.168.0.3"),
            "root@192.168.0.3"
        );
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(super::sanitize_name("Starbase2.local"), "starbase2-local");
        assert_eq!(super::sanitize_name("192.168.0.3"), "192-168-0-3");
    }
}
