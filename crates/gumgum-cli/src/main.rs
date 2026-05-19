use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use directories::ProjectDirs;
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
    host: String,
    #[arg(long)]
    user: Option<String>,
    #[arg(long)]
    root_domain: String,
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
            let test_domain = args
                .test_domain
                .clone()
                .unwrap_or_else(|| derive_test_domain(&args.root_domain));
            if cli.dry_run {
                let plan = SetupPlan::dry_run(args.host, args.user, args.root_domain, test_domain);
                print_value(cli.json, &plan)
            } else {
                let report = install_gumgumd(args, test_domain).await?;
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

async fn install_gumgumd(args: SetupArgs, test_domain: String) -> gumgum_core::Result<SetupReport> {
    let target = ssh_target(args.user.as_deref(), &args.host);
    let remote_gumgumd = build_remote_gumgumd(&target).await?;
    let install_mode = detect_install_mode(&target).await?;
    match install_mode {
        InstallMode::System => install_system_service(&target, &remote_gumgumd).await?,
        InstallMode::User => install_user_service(&target, &remote_gumgumd).await?,
    }
    ping_host(&args.host).await?;
    save_server(ServerRecord {
        name: args.host.clone(),
        host: args.host.clone(),
        root_domain: args.root_domain.clone(),
        test_domain: test_domain.clone(),
        health_url: format!("http://{}:7777/healthz", args.host),
    })?;
    Ok(SetupReport {
        ok: true,
        host: args.host.clone(),
        root_domain: args.root_domain,
        test_domain,
        service: "gumgumd".to_owned(),
        health_url: format!("http://{}:7777/healthz", args.host),
        actions: setup_actions(),
    })
}

#[derive(Clone, Copy, Debug)]
enum InstallMode {
    System,
    User,
}

async fn build_remote_gumgumd(target: &str) -> gumgum_core::Result<String> {
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
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg("cd /tmp/gumgum-src && cargo build -p gumgumd"),
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
            .arg(format!("mkdir -p ~/.local/bin ~/.local/share/gumgum ~/.config/systemd/user && install -m 0755 {remote_gumgumd} ~/.local/bin/gumgumd")),
    )
    .await?;
    let service = user_systemd_service();
    run_command(
        TokioCommand::new("ssh")
            .arg(target)
            .arg(format!("cat > ~/.config/systemd/user/gumgumd.service <<'EOF'\n{service}\nEOF\nsystemctl --user daemon-reload\nsystemctl --user enable --now gumgumd\nsystemctl --user restart gumgumd")),
    )
    .await
}

fn config_path() -> gumgum_core::Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "gumgum", "gumgum").ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not locate config directory",
        )
        .build()
    })?;
    Ok(dirs.config_dir().join("servers.json"))
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
ExecStart=%h/.local/bin/gumgumd
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target"#
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
}
