use anyhow::Result;
use axum::{Json, Router, routing::get};
use clap::{Args, Parser, Subcommand};
use gumgum_api::{
    PingReport, ServerListReport, ServerRecord, SetupPlan, SetupReport, not_configured_status,
    setup_actions,
};
use gumgum_core::{DoctorCheck, DoctorReport, ErrorCode, GumgumError, Subsystem};
use gumgum_manifest::{WorkerManifest, load_worker_path, validate_path};
use serde::Serialize;
use std::{fs, net::SocketAddr, path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command as TokioCommand;

#[derive(Debug, Parser)]
#[command(name = "gumgum")]
#[command(about = "GumGum.dev local cloud control plane")]
#[command(version)]
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
    Version,
    Init(InitArgs),
    Deploy(DeployArgs),
    Setup(SetupArgs),
    Server(ServerCommand),
    Schema(SchemaCommand),
    Daemon,
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
struct DeployArgs {
    #[arg(default_value = "gumgum.toml")]
    path: PathBuf,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "worker")]
    kind: InitKind,
    #[arg(long, default_value_t = 3000)]
    port: u16,
    #[arg(long)]
    root_domain: Option<String>,
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum InitKind {
    Workspace,
    Worker,
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
        Command::Version => {
            print_value(cli.json, &version_report());
        }
        Command::Init(args) => {
            let report = init_manifest(args, cli.dry_run)?;
            print_value(cli.json, &report);
        }
        Command::Deploy(args) => {
            let report = deploy_plan(args, cli.dry_run)?;
            print_value(cli.json, &report);
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
                    resolved.local,
                );
                print_value(cli.json, &plan)
            } else {
                let report = install_gumgumd(resolved, cli.json).await?;
                print_value(cli.json, &report)
            }
        }
        Command::Daemon => run_daemon().await?,
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

#[derive(Debug, Serialize)]
struct DeployPlanReport {
    ok: bool,
    dry_run: bool,
    path: String,
    worker: String,
    build_context: Option<String>,
    image: Option<String>,
    port: Option<u16>,
    routes: Vec<String>,
    message: String,
}

fn deploy_plan(args: DeployArgs, dry_run: bool) -> gumgum_core::Result<DeployPlanReport> {
    if !dry_run {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::NotImplemented,
            "gumgum deploy currently supports --dry-run only",
        )
        .next_command("gumgum --dry-run deploy")
        .build());
    }
    let manifest = load_worker_path(&args.path)?;
    Ok(deploy_plan_report(args.path, manifest))
}

fn deploy_plan_report(path: PathBuf, manifest: WorkerManifest) -> DeployPlanReport {
    DeployPlanReport {
        ok: true,
        dry_run: true,
        path: path.display().to_string(),
        worker: manifest.worker.name,
        build_context: manifest.worker.build_context,
        image: manifest.worker.image,
        port: manifest.worker.port,
        routes: manifest
            .ingress
            .into_iter()
            .map(|ingress| ingress.local_domain)
            .collect(),
        message: "validated worker manifest; no containers changed".to_owned(),
    }
}

#[derive(Debug, Serialize)]
struct InitReport {
    ok: bool,
    path: String,
    manifest_kind: &'static str,
    created: bool,
    files: Vec<String>,
    message: String,
}

fn init_manifest(args: InitArgs, dry_run: bool) -> gumgum_core::Result<InitReport> {
    let path = PathBuf::from("gumgum.toml");
    let name = args.name.unwrap_or_else(default_project_name);
    let root_domain = args.root_domain.or_else(|| {
        load_default_server()
            .ok()
            .flatten()
            .map(|server| server.root_domain)
    });
    let raw = match args.kind {
        InitKind::Workspace => workspace_manifest(&name, root_domain.as_deref()),
        InitKind::Worker => worker_manifest(&name, args.port, root_domain.as_deref()),
    };

    if path.exists() && !args.force {
        validate_path(&path)?;
        return Ok(InitReport {
            ok: true,
            path: path.display().to_string(),
            manifest_kind: match args.kind {
                InitKind::Workspace => "workspace",
                InitKind::Worker => "worker",
            },
            created: false,
            files: vec![path.display().to_string()],
            message: "gumgum.toml already exists; use --force to overwrite".to_owned(),
        });
    }

    let mut files = vec![path.display().to_string()];
    if matches!(args.kind, InitKind::Worker) {
        files.extend(scaffold_example_files(dry_run)?);
    }

    if !dry_run {
        fs::write(&path, raw).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write gumgum.toml",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        validate_path(&path)?;
    }

    Ok(InitReport {
        ok: true,
        path: path.display().to_string(),
        manifest_kind: match args.kind {
            InitKind::Workspace => "workspace",
            InitKind::Worker => "worker",
        },
        created: !dry_run,
        files,
        message: if dry_run {
            "would create gumgum.toml".to_owned()
        } else {
            "created gumgum.toml".to_owned()
        },
    })
}

fn default_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .map(|name| sanitize_name(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "hello".to_owned())
}

fn workspace_manifest(name: &str, root_domain: Option<&str>) -> String {
    let mut raw = format!("[workspace]\nname = \"{name}\"\nmembers = [\"apps/*\"]\n");
    if let Some(root_domain) = root_domain {
        raw.push_str(&format!("root_domain = \"{root_domain}\"\n"));
    }
    raw
}

fn worker_manifest(name: &str, port: u16, root_domain: Option<&str>) -> String {
    let local_domain = match root_domain {
        Some(root_domain) => format!("{name}.{}", root_domain.trim_start_matches("*.")),
        None => format!("{name}.local"),
    };
    format!(
        "[worker]\nname = \"{name}\"\nbuild_context = \".\"\nport = {port}\nhealth = \"/healthz\"\n\n[[ingress]]\nname = \"web\"\nprotocol = \"http\"\nlocal_domain = \"{local_domain}\"\n"
    )
}

fn scaffold_example_files(dry_run: bool) -> gumgum_core::Result<Vec<String>> {
    let files = vec!["Dockerfile".to_owned(), "server.py".to_owned()];
    if dry_run {
        return Ok(files);
    }

    write_if_missing(
        "Dockerfile",
        r#"FROM python:3.12-alpine
WORKDIR /app
COPY server.py .
ENV PORT=3000
EXPOSE 3000
CMD ["python", "server.py"]
"#,
    )?;
    write_if_missing(
        "server.py",
        r#"from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os

PORT = int(os.environ.get("PORT", "3000"))

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok":true}')
            return

        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        body = {"ok": True, "message": "Hello from GumGum.dev"}
        self.wfile.write(json.dumps(body).encode())

    def log_message(self, format, *args):
        return

HTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
"#,
    )?;
    Ok(files)
}

fn write_if_missing(path: &str, contents: &str) -> gumgum_core::Result<()> {
    if PathBuf::from(path).exists() {
        return Ok(());
    }
    fs::write(path, contents).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            format!("could not write {path}"),
        )
        .likely_cause(source.to_string())
        .build()
    })
}

#[derive(Debug, Serialize)]
struct VersionReport {
    ok: bool,
    version: &'static str,
    git_sha: &'static str,
    target: &'static str,
}

fn version_report() -> VersionReport {
    VersionReport {
        ok: true,
        version: option_env!("GUMGUM_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
        git_sha: option_env!("GUMGUM_BUILD_SHA").unwrap_or("unknown"),
        target: option_env!("GUMGUM_BUILD_TARGET").unwrap_or("unknown"),
    }
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

async fn wait_for_ping(host: &str) -> gumgum_core::Result<PingReport> {
    let mut last_error = None;
    for _ in 0..20 {
        match ping_host(host).await {
            Ok(report) if report.ok => return Ok(report),
            Ok(report) => {
                last_error = Some(format!("gumgumd health returned ok={}", report.ok));
            }
            Err(err) => {
                let report = err.to_report();
                last_error = Some(report.likely_cause.unwrap_or(report.message));
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(
        GumgumError::structured(Subsystem::Api, ErrorCode::Io, "failed to reach gumgumd")
            .likely_cause(last_error.unwrap_or_else(|| "health check timed out".to_owned()))
            .next_command("gumgum setup 127.0.0.1 --root-domain <domain>")
            .build(),
    )
}

async fn run_daemon() -> gumgum_core::Result<()> {
    let app = Router::new()
        .route("/healthz", get(daemon_healthz))
        .route("/v0/status", get(daemon_status));
    let addr = SocketAddr::from(([0, 0, 0, 0], 7777));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "could not bind gumgum daemon",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    tracing::info!(%addr, "gumgum daemon listening");
    axum::serve(listener, app).await.map_err(|source| {
        GumgumError::structured(Subsystem::Api, ErrorCode::Io, "gumgum daemon failed")
            .likely_cause(source.to_string())
            .build()
    })
}

async fn daemon_healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "service": "gumgumd" }))
}

async fn daemon_status() -> Json<gumgum_core::StatusReport> {
    Json(not_configured_status())
}

async fn install_gumgumd(setup: ResolvedSetup, quiet: bool) -> gumgum_core::Result<SetupReport> {
    progress(quiet, "resolving setup target");
    if setup.local {
        progress(
            quiet,
            "installing local binary into ~/.gumgum/bin and daemon service into ~/.gumgum/daemon",
        );
        install_local_user_service(quiet).await?;
    } else {
        let target = ssh_target(setup.user.as_deref(), &setup.host);
        progress(quiet, format!("running remote bootstrap on {target}"));
        run_remote_setup(&target, &setup, quiet).await?;
    }
    progress(quiet, "checking gumgumd health");
    wait_for_ping(&setup.host).await?;
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
        actions: setup_actions(setup.local),
    })
}

async fn install_local_user_service(quiet: bool) -> gumgum_core::Result<()> {
    let gumgum = std::env::current_exe().map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not locate running gumgum binary",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
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
    fs::create_dir_all(format!("{home}/.gumgum/bin")).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not create ~/.gumgum/bin",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let installed_gumgum = PathBuf::from(format!("{home}/.gumgum/bin/gumgum"));
    if gumgum != installed_gumgum {
        fs::copy(&gumgum, &installed_gumgum).map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "could not install local gumgumd",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    }
    run_command_streaming(
        TokioCommand::new("chmod")
            .arg("0755")
            .arg(&installed_gumgum),
        quiet,
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
    run_command_streaming(
        TokioCommand::new("ln")
            .arg("-sf")
            .arg(format!("{home}/.gumgum/daemon/gumgumd.service"))
            .arg(format!("{home}/.config/systemd/user/gumgumd.service")),
        quiet,
    )
    .await?;
    run_command_streaming(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("daemon-reload"),
        quiet,
    )
    .await?;
    run_command_streaming(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("enable")
            .arg("--now")
            .arg("gumgumd"),
        quiet,
    )
    .await?;
    run_command_streaming(
        TokioCommand::new("systemctl")
            .arg("--user")
            .arg("restart")
            .arg("gumgumd"),
        quiet,
    )
    .await
}

async fn run_remote_setup(
    target: &str,
    setup: &ResolvedSetup,
    quiet: bool,
) -> gumgum_core::Result<()> {
    let remote_setup = format!(
        "~/.gumgum/bin/gumgum setup --name {} --root-domain {} --test-domain {}{}",
        shell_quote(&setup.name),
        shell_quote(&setup.root_domain),
        shell_quote(&setup.test_domain),
        if quiet { " --json" } else { "" }
    );
    let script = format!(
        "set -e; primary=https://get.gumgum.dev; fallback=https://get-gumgum-dev.abstractmachines.workers.dev; tmp=$(mktemp); trap 'rm -f $tmp' EXIT; if command -v curl >/dev/null 2>&1; then if curl -fsSL -o $tmp $primary; then GUMGUM_NO_PATH=1 sh $tmp; else echo 'primary installer URL failed; retrying workers.dev fallback' >&2; curl -fsSL -o $tmp $fallback; GUMGUM_BASE_URL=$fallback GUMGUM_NO_PATH=1 sh $tmp; fi; elif command -v wget >/dev/null 2>&1; then if wget -q -O $tmp $primary; then GUMGUM_NO_PATH=1 sh $tmp; else echo 'primary installer URL failed; retrying workers.dev fallback' >&2; wget -q -O $tmp $fallback; GUMGUM_BASE_URL=$fallback GUMGUM_NO_PATH=1 sh $tmp; fi; else echo 'curl or wget is required' >&2; exit 1; fi; {remote_setup}"
    );
    run_command_streaming(TokioCommand::new("ssh").arg(target).arg(script), quiet).await
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

async fn run_command_streaming(cmd: &mut TokioCommand, quiet: bool) -> gumgum_core::Result<()> {
    if quiet {
        return run_command(cmd).await;
    }
    let status = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "failed to run setup command",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "setup command failed")
                .likely_cause(format!("exit status {status}"))
                .next_command("gumgum setup <host> --root-domain <domain> --dry-run")
                .build(),
        )
    }
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn user_systemd_service() -> &'static str {
    r#"[Unit]
Description=GumGum.dev daemon
After=default.target

[Service]
Type=simple
ExecStart=%h/.gumgum/bin/gumgum daemon
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
