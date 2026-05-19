use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use gumgum_api::{PingReport, SetupPlan, SetupReport, not_configured_status, setup_actions};
use gumgum_core::{DoctorCheck, DoctorReport, ErrorCode, GumgumError, Subsystem};
use gumgum_manifest::validate_path;
use serde::Serialize;
use std::path::{Path, PathBuf};
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
                let report = ping_host(&ssh_target(args.user.as_deref(), &host), host).await?;
                print_value(cli.json, &report)
            } else {
                print_value(cli.json, &not_configured_status())
            }
        }
        Command::Ping(args) => {
            let report =
                ping_host(&ssh_target(args.user.as_deref(), &args.host), args.host).await?;
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

async fn ping_host(target: &str, host: String) -> gumgum_core::Result<PingReport> {
    let health_raw = run_command_stdout(
        TokioCommand::new("ssh")
            .arg(target)
            .arg("curl -fsS http://127.0.0.1:7777/healthz"),
    )
    .await?;
    let health: serde_json::Value = serde_json::from_str(&health_raw).map_err(|source| {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd returned invalid JSON",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let service_raw = run_command_stdout(
        TokioCommand::new("ssh")
            .arg(target)
            .arg("systemctl --user is-active gumgumd 2>/dev/null || systemctl is-active gumgumd 2>/dev/null || true"),
    )
    .await?;
    Ok(PingReport {
        ok: health
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        host,
        health_url: "http://127.0.0.1:7777/healthz".to_owned(),
        service_active: Some(service_raw.trim() == "active"),
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
    run_command(
        TokioCommand::new("ssh")
            .arg(&target)
            .arg("curl -fsS http://127.0.0.1:7777/healthz >/dev/null"),
    )
    .await?;
    Ok(SetupReport {
        ok: true,
        host: args.host,
        root_domain: args.root_domain,
        test_domain,
        service: "gumgumd".to_owned(),
        health_url: "http://127.0.0.1:7777/healthz".to_owned(),
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

async fn run_command_stdout(cmd: &mut TokioCommand) -> gumgum_core::Result<String> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "failed to run remote command",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(
        GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "remote command failed")
            .likely_cause(if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            })
            .build(),
    )
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
