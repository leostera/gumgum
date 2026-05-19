use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    routing::{get, post},
};
use clap::{Args, Parser, Subcommand};
use gumgum_api::{
    DeployApplyReport, DeployRequest, GraphReport, LogsReport, PingReport, ServerListReport,
    ServerRecord, SetupPlan, SetupReport, not_configured_status, setup_actions,
};
use gumgum_core::{DoctorCheck, DoctorReport, ErrorCode, GumgumError, Subsystem};
use gumgum_manifest::{WorkerManifest, load_worker_path, validate_path};
use rusqlite::{Connection, params};
use serde::Serialize;
use std::{fs, net::SocketAddr, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
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
    Config(ConfigArgs),
    Init(InitArgs),
    Deploy(DeployArgs),
    Logs(LogsArgs),
    Graph(GraphArgs),
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
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    List,
    Get { key: String },
    Set { key: String, value: String },
}

#[derive(Debug, Args)]
struct DeployArgs {
    #[arg(default_value = "gumgum.toml")]
    path: PathBuf,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    prod: bool,
}

#[derive(Debug, Args)]
struct GraphArgs {
    #[arg(long)]
    host: Option<String>,
}

#[derive(Debug, Args)]
struct LogsArgs {
    #[arg(default_value = "gumgum.toml")]
    path: PathBuf,
    #[arg(long, short)]
    follow: bool,
    #[arg(long)]
    host: Option<String>,
    #[arg(long, default_value_t = 100)]
    tail: u32,
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
    namespace: Option<String>,
    #[arg(long = "zone")]
    zones: Vec<String>,
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
    Config(ServerConfigArgs),
}

#[derive(Debug, Args)]
struct ServerConfigArgs {
    name: String,
    #[command(subcommand)]
    command: ConfigSubcommand,
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
        Command::Config(args) => {
            let report = config_command(None, args.command)?;
            print_value(cli.json, &report);
        }
        Command::Init(args) => {
            let report = init_manifest(args, cli.dry_run)?;
            print_value(cli.json, &report);
        }
        Command::Deploy(args) => {
            let report = deploy(args, cli.dry_run, cli.json).await?;
            print_value(cli.json, &report);
        }
        Command::Logs(args) => {
            logs(args, cli.json).await?;
        }
        Command::Graph(args) => {
            graph(args, cli.json).await?;
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
            ServerSubcommand::Config(args) => {
                let report = config_command(Some(args.name), args.command)?;
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
struct ConfigReport {
    ok: bool,
    scope: String,
    values: serde_json::Map<String, serde_json::Value>,
    message: String,
}

fn config_command(
    server_name: Option<String>,
    command: ConfigSubcommand,
) -> gumgum_core::Result<ConfigReport> {
    let path = match &server_name {
        Some(name) => server_config_path(name)?,
        None => local_config_path()?,
    };
    let scope = server_name
        .map(|name| format!("server:{name}"))
        .unwrap_or_else(|| "local".to_owned());
    let mut values = load_config_map(&path)?;
    match command {
        ConfigSubcommand::List => Ok(ConfigReport {
            ok: true,
            scope,
            values,
            message: "config values".to_owned(),
        }),
        ConfigSubcommand::Get { key } => {
            let mut selected = serde_json::Map::new();
            if let Some(value) = values.get(&key) {
                selected.insert(key, value.clone());
            }
            Ok(ConfigReport {
                ok: true,
                scope,
                values: selected,
                message: "config value".to_owned(),
            })
        }
        ConfigSubcommand::Set { key, value } => {
            values.insert(key.clone(), serde_json::Value::String(value));
            save_config_map(&path, &values)?;
            let mut selected = serde_json::Map::new();
            selected.insert(key.clone(), values.get(&key).cloned().unwrap());
            Ok(ConfigReport {
                ok: true,
                scope,
                values: selected,
                message: "config value saved".to_owned(),
            })
        }
    }
}

#[derive(Debug, Serialize)]
struct DeployReport {
    ok: bool,
    dry_run: bool,
    path: String,
    worker: String,
    host: Option<String>,
    build_context: Option<String>,
    image: String,
    container: String,
    port: u16,
    routes: Vec<String>,
    health_url: Option<String>,
    message: String,
}

async fn deploy(args: DeployArgs, dry_run: bool, quiet: bool) -> gumgum_core::Result<DeployReport> {
    let manifest = load_worker_path(&args.path)?;
    let server = match args.host {
        Some(host) => Some(ServerRecord {
            name: sanitize_name(&host),
            host: host.clone(),
            root_domain: String::new(),
            test_domain: String::new(),
            health_url: format!("http://{host}:7777/healthz"),
        }),
        None => load_default_server()?,
    };
    let prod = args.prod;
    let report = deploy_report(args.path.clone(), &manifest, server.as_ref(), dry_run, prod);
    if dry_run {
        return Ok(report);
    }
    let server = server.ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            "no GumGum.dev server configured",
        )
        .next_command("gumgum setup <host> --root-domain <domain>")
        .build()
    })?;
    run_remote_deploy(&server, &manifest, &report, quiet).await?;
    if let Some(route) = report.health_url.as_deref().and_then(route_from_health_url) {
        configure_client_host_route(route, &server.host, quiet).await?;
    }
    Ok(DeployReport {
        ok: true,
        dry_run: false,
        message: format!("deployed {} to {}", report.worker, server.host),
        ..report
    })
}

fn deploy_report(
    path: PathBuf,
    manifest: &WorkerManifest,
    server: Option<&ServerRecord>,
    dry_run: bool,
    prod: bool,
) -> DeployReport {
    let worker = manifest.worker.name.clone();
    let image = format!("127.0.0.1:55000/gumgum/{worker}:latest");
    let container = format!("gumgum-{}", sanitize_name(&worker));
    let routes = derived_routes(manifest, server, prod);
    let health_url = derived_routes(manifest, server, false)
        .first()
        .map(|route| {
            let display_route = server
                .map(|server| {
                    let root_suffix = format!(".{}", server.root_domain);
                    if route.ends_with(&root_suffix) {
                        format!(
                            "{}.{test_domain}",
                            route.trim_end_matches(&root_suffix),
                            test_domain = server.test_domain
                        )
                    } else {
                        route.clone()
                    }
                })
                .unwrap_or_else(|| route.clone());
            format!(
                "http://{display_route}{}",
                manifest.worker.health.as_deref().unwrap_or("/healthz")
            )
        });
    DeployReport {
        ok: true,
        dry_run,
        path: path.display().to_string(),
        worker,
        host: server.map(|server| server.host.clone()),
        build_context: manifest.worker.build_context.clone(),
        image,
        container,
        port: manifest.worker.port.unwrap_or(3000),
        routes,
        health_url,
        message: if dry_run {
            format!(
                "validated worker manifest for {} deploy; no containers changed",
                if prod { "prod" } else { "test" }
            )
        } else {
            "deployment pending".to_owned()
        },
    }
}

async fn graph(args: GraphArgs, json: bool) -> gumgum_core::Result<()> {
    let server = match args.host {
        Some(host) => host,
        None => {
            load_default_server()?
                .ok_or_else(|| {
                    GumgumError::structured(
                        Subsystem::Config,
                        ErrorCode::InvalidArgs,
                        "no GumGum.dev server configured",
                    )
                    .next_command("gumgum setup <host> --root-domain <domain>")
                    .build()
                })?
                .host
        }
    };
    let url = format!("http://{server}:7777/v0/graph");
    let report: GraphReport = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "failed to call gumgumd graph API",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .error_for_status()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "gumgumd graph API returned an error",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .json()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "gumgumd graph API returned invalid JSON",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if json {
        print_value(true, &report);
    } else {
        println!("{}", report.graph);
    }
    Ok(())
}

async fn logs(args: LogsArgs, quiet: bool) -> gumgum_core::Result<()> {
    if args.follow {
        return Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::NotImplemented,
            "gumgum logs -f will stream through gumgumd in a later release",
        )
        .next_command("gumgum logs")
        .build());
    }
    let manifest = load_worker_path(&args.path)?;
    let server = match args.host {
        Some(host) => host,
        None => {
            load_default_server()?
                .ok_or_else(|| {
                    GumgumError::structured(
                        Subsystem::Config,
                        ErrorCode::InvalidArgs,
                        "no GumGum.dev server configured",
                    )
                    .next_command("gumgum setup <host> --root-domain <domain>")
                    .build()
                })?
                .host
        }
    };
    let container = format!("gumgum-{}", sanitize_name(&manifest.worker.name));
    let url = format!(
        "http://{server}:7777/v0/logs/{container}?tail={}",
        args.tail
    );
    let report: LogsReport = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "failed to call gumgumd logs API",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .error_for_status()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "gumgumd logs API returned an error",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .json()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "gumgumd logs API returned invalid JSON",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if quiet {
        print_value(true, &report);
    } else {
        print!("{}", report.logs);
    }
    Ok(())
}

async fn run_remote_deploy(
    server: &ServerRecord,
    manifest: &WorkerManifest,
    report: &DeployReport,
    quiet: bool,
) -> gumgum_core::Result<()> {
    let context = manifest.worker.build_context.as_deref().unwrap_or(".");
    let host = &server.host;
    let local_image = report.image.replacen("127.0.0.1", "localhost", 1);
    let route = deploy_route(report, server);

    wait_for_remote_registry(host, quiet).await?;
    progress(quiet, format!("opening registry tunnel to {host}"));
    let mut tunnel = TokioCommand::new("ssh")
        .arg("-N")
        .arg("-L")
        .arg("55000:127.0.0.1:55000")
        .arg(host)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "could not open registry tunnel",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    progress(quiet, format!("building image {local_image} locally"));
    let build_result = run_command_streaming(
        TokioCommand::new("docker")
            .arg("build")
            .arg("--platform")
            .arg("linux/amd64")
            .arg("-t")
            .arg(&local_image)
            .arg(context),
        quiet,
    )
    .await;
    if build_result.is_ok() {
        progress(quiet, "pushing image to GumGum.dev registry");
        run_command_streaming(
            TokioCommand::new("docker").arg("push").arg(&local_image),
            quiet,
        )
        .await?;
    }
    let _ = tunnel.kill().await;
    build_result?;

    progress(
        quiet,
        format!("asking gumgumd on {host} to reconcile {}", report.worker),
    );
    let request = DeployRequest {
        worker: report.worker.clone(),
        image: report.image.clone(),
        container: report.container.clone(),
        route: route.clone(),
        port: report.port,
        health: manifest
            .worker
            .health
            .clone()
            .unwrap_or_else(|| "/healthz".to_owned()),
    };
    apply_deploy_via_daemon(host, &request).await?;
    verify_route(
        server,
        &route,
        manifest.worker.health.as_deref().unwrap_or("/healthz"),
        quiet,
    )
    .await
}

async fn apply_deploy_via_daemon(
    host: &str,
    request: &DeployRequest,
) -> gumgum_core::Result<DeployApplyReport> {
    let url = format!("http://{host}:7777/v0/deploy");
    let response = reqwest::Client::new()
        .post(&url)
        .json(request)
        .send()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "failed to call gumgumd deploy API",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if !response.status().is_success() {
        return Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd deploy API returned an error",
        )
        .likely_cause(response.status().to_string())
        .build());
    }
    response.json().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd deploy API returned invalid JSON",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

async fn ensure_local_platform(quiet: bool) -> gumgum_core::Result<()> {
    progress(quiet, "ensuring GumGum.dev Docker network");
    run_command_streaming(
        TokioCommand::new("sh").arg("-c").arg("docker network inspect gumgum-network >/dev/null 2>&1 || docker network create gumgum-network >/dev/null"),
        quiet,
    )
    .await?;

    ensure_local_registry(quiet).await?;
    ensure_local_dnsmasq(quiet).await?;
    ensure_local_caddy(quiet).await
}

async fn ensure_local_registry(quiet: bool) -> gumgum_core::Result<()> {
    progress(quiet, "ensuring GumGum.dev registry container");
    run_command_streaming(
        TokioCommand::new("sh").arg("-c").arg("docker inspect gumgum-registry >/dev/null 2>&1 && docker start gumgum-registry >/dev/null || docker run -d --name gumgum-registry --restart unless-stopped --network gumgum-network -p 127.0.0.1:55000:5000 registry:2 >/dev/null"),
        quiet,
    )
    .await
}

async fn ensure_local_dnsmasq(quiet: bool) -> gumgum_core::Result<()> {
    progress(quiet, "ensuring GumGum.dev dnsmasq container");
    let script = "set -e; mkdir -p ~/.gumgum/dnsmasq; upstream=$(ip route 2>/dev/null | awk '/^default/ {print $3; exit}'); if [ -z \"$upstream\" ]; then upstream=$(awk '/^nameserver/ {print $2; exit}' /etc/resolv.conf 2>/dev/null || true); fi; [ -n \"$upstream\" ] || upstream=1.1.1.1; tmp=$(mktemp); { printf 'listen-address=0.0.0.0\nbind-interfaces\nno-resolv\nserver=%s\ncache-size=10000\n' \"$upstream\"; if [ -f ~/.gumgum/dnsmasq/dnsmasq.conf ]; then grep '^address=/' ~/.gumgum/dnsmasq/dnsmasq.conf || true; fi; } > $tmp; mv $tmp ~/.gumgum/dnsmasq/dnsmasq.conf; if docker inspect gumgum-dnsmasq >/dev/null 2>&1; then docker start gumgum-dnsmasq >/dev/null; docker restart gumgum-dnsmasq >/dev/null; elif docker ps --format '{{.Ports}}' | grep -qE '(^|, )0\\.0\\.0\\.0:53->|(^|, )[^ ]*:53->|:53->'; then echo 'warning: port 53 is already in use; gumgum-dnsmasq not started' >&2; else docker run -d --name gumgum-dnsmasq --restart unless-stopped --network gumgum-network -p 53:53/tcp -p 53:53/udp -v $HOME/.gumgum/dnsmasq/dnsmasq.conf:/etc/dnsmasq.conf:ro jpillora/dnsmasq:latest >/dev/null; fi";
    run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
}

async fn ensure_local_caddy(quiet: bool) -> gumgum_core::Result<()> {
    progress(quiet, "ensuring GumGum.dev Caddy container");
    let script = "set -e; if docker inspect gumgum-caddy >/dev/null 2>&1; then docker start gumgum-caddy >/dev/null; elif docker ps --format '{{.Ports}}' | grep -qE '(^|, )0\\.0\\.0\\.0:(80|443)->|(^|, )[^ ]*:(80|443)->'; then echo 'warning: ports 80/443 are already in use; gumgum-caddy not started' >&2; else docker run -d --name gumgum-caddy --restart unless-stopped --network gumgum-network -p 80:80 -p 443:443 -v /var/run/docker.sock:/var/run/docker.sock:ro lucaslorentz/caddy-docker-proxy:2.9-alpine >/dev/null; fi";
    run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
}

async fn wait_for_remote_registry(host: &str, quiet: bool) -> gumgum_core::Result<()> {
    progress(
        quiet,
        format!("checking GumGum.dev registry managed by daemon on {host}"),
    );
    let script = "for i in $(seq 1 20); do if docker inspect -f '{{.State.Running}}' gumgum-registry 2>/dev/null | grep -q true; then exit 0; fi; sleep 0.5; done; echo 'gumgum-registry is not running; is gumgumd active?' >&2; exit 1";
    run_command_streaming(TokioCommand::new("ssh").arg(host).arg(script), quiet).await
}

fn deploy_route(report: &DeployReport, _server: &ServerRecord) -> String {
    report
        .routes
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{}.local", report.worker))
}

fn derived_routes(
    manifest: &WorkerManifest,
    server: Option<&ServerRecord>,
    prod: bool,
) -> Vec<String> {
    let worker = sanitize_name(&manifest.worker.name);
    let project = manifest
        .project
        .as_ref()
        .map(|project| sanitize_name(&project.namespace))
        .unwrap_or_else(default_project_name);
    let Some(server) = server else {
        return manifest
            .ingress
            .iter()
            .map(|ingress| ingress.local_domain.clone())
            .collect();
    };

    if prod {
        let mut routes = vec![format!("{worker}.{project}.{}", server.root_domain)];
        routes.extend(
            manifest
                .zone
                .iter()
                .map(|zone| format!("{worker}.{}", zone.name.trim_start_matches("*."))),
        );
        routes
    } else {
        vec![format!("{worker}.{project}.{}", server.test_domain)]
    }
}

async fn verify_route(
    server: &ServerRecord,
    route: &str,
    health: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    progress(quiet, format!("verifying http://{route}{health}"));
    let url = format!("http://{}{health}", server.host);
    let status = TokioCommand::new("curl")
        .arg("-fsS")
        .arg("-H")
        .arg(format!("Host: {route}"))
        .arg(url)
        .status()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "failed to verify deployed route",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "deployed route did not respond",
        )
        .likely_cause(format!("curl exited with {status}"))
        .next_command(format!(
            "curl -H 'Host: {route}' http://{}{health}",
            server.host
        ))
        .build())
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
    let namespace = args.namespace.unwrap_or_else(|| name.clone());
    let raw = match args.kind {
        InitKind::Workspace => workspace_manifest(&name, root_domain.as_deref()),
        InitKind::Worker => worker_manifest(&name, &namespace, args.port, &args.zones),
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

fn worker_manifest(name: &str, namespace: &str, port: u16, zones: &[String]) -> String {
    let mut raw = format!(
        "[project]\nnamespace = \"{namespace}\"\n\n[worker]\nname = \"{name}\"\nbuild_context = \".\"\nport = {port}\nhealth = \"/healthz\"\n"
    );
    for zone in zones {
        raw.push_str(&format!("\n[[zone]]\nname = \"{zone}\"\n"));
    }
    raw
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
        print("%s - %s" % (self.address_string(), format % args), flush=True)

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

async fn configure_host_dns(test_domain: &str, quiet: bool) -> gumgum_core::Result<()> {
    progress(quiet, format!("configuring host DNS for *.{test_domain}"));
    let domain = shell_escape_plain(test_domain);
    let script = format!(
        "set -e; ip=$(hostname -I 2>/dev/null | awk '{{print $1}}'); [ -n \"$ip\" ] || ip=127.0.0.1; if [ -w $HOME/.gumgum/dnsmasq/dnsmasq.conf ]; then if ! grep -q '^address=/{domain}/' $HOME/.gumgum/dnsmasq/dnsmasq.conf; then printf '\n# GumGum.dev test domain\naddress=/{domain}/%s\n' \"$ip\" >> $HOME/.gumgum/dnsmasq/dnsmasq.conf; fi; if docker inspect gumgum-dnsmasq >/dev/null 2>&1; then docker restart gumgum-dnsmasq >/dev/null; fi; fi; if docker inspect dnsmasq >/dev/null 2>&1 && [ -w /apps/fleet/gateway/dnsmasq/dnsmasq.conf ]; then if ! grep -q '^address=/{domain}/' /apps/fleet/gateway/dnsmasq/dnsmasq.conf; then printf '\n# GumGum.dev test domain\naddress=/{domain}/%s\n' \"$ip\" >> /apps/fleet/gateway/dnsmasq/dnsmasq.conf; fi; docker restart dnsmasq >/dev/null; fi"
    );
    run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
}

fn route_from_health_url(url: &str) -> Option<&str> {
    url.strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .and_then(|rest| rest.split('/').next())
}

async fn configure_client_host_route(
    route: &str,
    host: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    if std::env::consts::OS != "macos" {
        return Ok(());
    }
    progress(
        quiet,
        format!("configuring local hosts entry for {route} -> {host}"),
    );
    let route = shell_escape_plain(route);
    let host = shell_escape_plain(host);
    let script = format!(
        "set -e; if [ ! -t 0 ] && ! sudo -n true 2>/dev/null; then echo 'warning: run this to enable browser/curl route: printf \"{host} {route}\\n\" | sudo tee -a /etc/hosts' >&2; exit 0; fi; if ! grep -q '[[:space:]]{route}$' /etc/hosts; then printf '{host} {route}\n' | sudo tee -a /etc/hosts >/dev/null; fi; sudo dscacheutil -flushcache; sudo killall -HUP mDNSResponder 2>/dev/null || true"
    );
    run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
}

async fn configure_client_resolver(
    test_domain: &str,
    host: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    match std::env::consts::OS {
        "macos" => {
            progress(
                quiet,
                format!("configuring local resolver for {test_domain} -> {host}"),
            );
            let script = format!(
                "set -e; if [ ! -t 0 ] && ! sudo -n true 2>/dev/null; then echo 'warning: run this to enable browser DNS: sudo mkdir -p /etc/resolver && printf nameserver\\ {host}\\\\n | sudo tee /etc/resolver/{domain}' >&2; exit 0; fi; sudo mkdir -p /etc/resolver; printf 'nameserver {host}\n' | sudo tee /etc/resolver/{domain} >/dev/null; sudo dscacheutil -flushcache",
                host = shell_escape_plain(host),
                domain = shell_escape_plain(test_domain)
            );
            run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
        }
        _ => {
            progress(
                quiet,
                format!(
                    "skipping automatic resolver setup on {}; configure {test_domain} to use nameserver {host}",
                    std::env::consts::OS
                ),
            );
            Ok(())
        }
    }
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

#[derive(Clone)]
struct DaemonState {
    graph_path: Arc<PathBuf>,
}

async fn run_daemon() -> gumgum_core::Result<()> {
    ensure_local_platform(false).await?;
    let graph_path = gumgum_root()?.join("graph.sqlite");
    init_graph_db(&graph_path)?;
    let state = DaemonState {
        graph_path: Arc::new(graph_path),
    };
    let app = Router::new()
        .route("/healthz", get(daemon_healthz))
        .route("/v0/status", get(daemon_status))
        .route("/v0/deploy", post(daemon_deploy))
        .route("/v0/graph", get(daemon_graph))
        .route("/v0/logs/{container}", get(daemon_logs))
        .with_state(state);
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

async fn daemon_graph(State(state): State<DaemonState>) -> Json<GraphReport> {
    let path = (*state.graph_path).clone();
    let graph = tokio::task::spawn_blocking(move || render_mermaid_graph(&path))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| "graph TD\n  gumgumd[gumgumd]\n".to_owned());
    Json(GraphReport {
        ok: true,
        format: "mermaid".to_owned(),
        graph,
    })
}

fn render_mermaid_graph(path: &PathBuf) -> gumgum_core::Result<String> {
    init_graph_db(path)?;
    let conn = Connection::open(path).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not open graph database",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let mut graph = "graph TD\n  gumgumd[gumgumd]\n  registry[gumgum-registry]\n  dnsmasq[gumgum-dnsmasq]\n  caddy[gumgum-caddy]\n  gumgumd --> registry\n  gumgumd --> dnsmasq\n  gumgumd --> caddy\n".to_owned();
    let mut stmt = conn
        .prepare("SELECT worker, image, container, route FROM desired_deployments ORDER BY worker")
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not query graph database",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not read graph rows",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    for row in rows {
        let (worker, image, container, route) = row.map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not decode graph row",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        let worker_id = mermaid_id(&format!("worker-{worker}"));
        let image_id = mermaid_id(&format!("image-{worker}"));
        let container_id = mermaid_id(&format!("container-{container}"));
        let route_id = mermaid_id(&format!("route-{route}"));
        graph.push_str(&format!("  {worker_id}[worker: {worker}]\n"));
        graph.push_str(&format!("  {image_id}[image: {image}]\n"));
        graph.push_str(&format!("  {container_id}[container: {container}]\n"));
        graph.push_str(&format!("  {route_id}[route: {route}]\n"));
        graph.push_str(&format!("  gumgumd --> {worker_id}\n  {worker_id} --> {image_id}\n  {worker_id} --> {container_id}\n  {worker_id} --> {route_id}\n  registry --> {image_id}\n  caddy --> {route_id}\n  {route_id} --> {container_id}\n"));
    }
    Ok(graph)
}

fn mermaid_id(value: &str) -> String {
    sanitize_name(value).replace('-', "_")
}

#[derive(Debug, serde::Deserialize)]
struct LogsQuery {
    tail: Option<u32>,
}

async fn daemon_logs(
    AxumPath(container): AxumPath<String>,
    Query(query): Query<LogsQuery>,
) -> Json<LogsReport> {
    let tail = query.tail.unwrap_or(100);
    let output = TokioCommand::new("docker")
        .arg("logs")
        .arg("--tail")
        .arg(tail.to_string())
        .arg(&container)
        .output()
        .await;
    let logs = match output {
        Ok(output) => {
            let mut logs = String::new();
            logs.push_str(&String::from_utf8_lossy(&output.stdout));
            logs.push_str(&String::from_utf8_lossy(&output.stderr));
            logs
        }
        Err(source) => format!("failed to read logs: {source}\n"),
    };
    Json(LogsReport {
        ok: true,
        container,
        tail,
        logs,
    })
}

async fn daemon_deploy(
    State(state): State<DaemonState>,
    Json(request): Json<DeployRequest>,
) -> Json<DeployApplyReport> {
    let path = (*state.graph_path).clone();
    let request_for_db = request.clone();
    let materialized =
        tokio::task::spawn_blocking(move || materialize_deploy(&path, &request_for_db))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
    let (changed, actions) = reconcile_deploy(&request).await.unwrap_or_else(|error| {
        (
            false,
            vec![format!("reconcile failed: {}", error.to_report().message)],
        )
    });
    Json(DeployApplyReport {
        ok: materialized,
        worker: request.worker,
        materialized,
        changed,
        actions,
        message: "desired deployment materialized and reconciled".to_owned(),
    })
}

fn init_graph_db(path: &PathBuf) -> gumgum_core::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not create graph directory",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    }
    let conn = Connection::open(path).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not open graph database",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS desired_deployments (
            worker TEXT PRIMARY KEY,
            image TEXT NOT NULL,
            container TEXT NOT NULL,
            route TEXT NOT NULL,
            port INTEGER NOT NULL,
            health TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not initialize graph database",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

fn materialize_deploy(path: &PathBuf, request: &DeployRequest) -> gumgum_core::Result<bool> {
    init_graph_db(path)?;
    let mut conn = Connection::open(path).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not open graph database",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let tx = conn.transaction().map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not begin graph transaction",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    tx.execute(
        "INSERT INTO desired_deployments (worker, image, container, route, port, health, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
         ON CONFLICT(worker) DO UPDATE SET
           image=excluded.image,
           container=excluded.container,
           route=excluded.route,
           port=excluded.port,
           health=excluded.health,
           updated_at=CURRENT_TIMESTAMP",
        params![request.worker, request.image, request.container, request.route, request.port, request.health],
    )
    .map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not materialize deployment")
            .likely_cause(source.to_string())
            .build()
    })?;
    tx.commit().map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not commit graph transaction",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    Ok(true)
}

async fn reconcile_deploy(request: &DeployRequest) -> gumgum_core::Result<(bool, Vec<String>)> {
    let mut actions = Vec::new();
    let inspect = TokioCommand::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg("{{.Config.Image}} {{index .Config.Labels \"caddy\"}} {{index .Config.Labels \"caddy.reverse_proxy\"}}")
        .arg(&request.container)
        .output()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "could not inspect deployment container",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    let current = String::from_utf8_lossy(&inspect.stdout).trim().to_owned();
    let expected_proxy = format!("{{{{upstreams {}}}}}", request.port);
    let expected = format!("{} {} {}", request.image, request.route, expected_proxy);
    let route_label = format!("caddy={}", request.route);
    if inspect.status.success() && current == expected {
        actions.push("container already matches desired image".to_owned());
        return Ok((false, actions));
    }
    actions.push(format!("pull {}", request.image));
    run_command_streaming(
        TokioCommand::new("docker").arg("pull").arg(&request.image),
        false,
    )
    .await?;
    let network = if docker_running("gumgum-caddy").await {
        "gumgum-network"
    } else {
        "caddy-network"
    };
    actions.push(format!("recreate {}", request.container));
    let _ = run_command_streaming(
        TokioCommand::new("docker")
            .arg("rm")
            .arg("-f")
            .arg(&request.container),
        true,
    )
    .await;
    run_command_streaming(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&request.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg(network)
            .arg("--label")
            .arg(route_label)
            .arg("--label")
            .arg(format!(
                "caddy.reverse_proxy={{{{upstreams {}}}}}",
                request.port
            ))
            .arg("--label")
            .arg("caddy.tls=internal")
            .arg(&request.image),
        false,
    )
    .await?;
    wait_for_container_health(&request.container, request.port, &request.health).await?;
    Ok((true, actions))
}

async fn docker_running(name: &str) -> bool {
    TokioCommand::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg("{{.State.Running}}")
        .arg(name)
        .output()
        .await
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        })
        .unwrap_or(false)
}

async fn wait_for_container_health(
    container: &str,
    port: u16,
    health: &str,
) -> gumgum_core::Result<()> {
    for _ in 0..20 {
        let output = TokioCommand::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}")
            .arg(container)
            .output()
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Setup,
                    ErrorCode::Io,
                    "could not inspect deployment IP",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        let ip = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !ip.is_empty() {
            let url = format!("http://{ip}:{port}{health}");
            if reqwest::get(&url)
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(GumgumError::structured(
        Subsystem::Api,
        ErrorCode::Io,
        "deployment container did not become healthy",
    )
    .build())
}

async fn install_gumgumd(setup: ResolvedSetup, quiet: bool) -> gumgum_core::Result<SetupReport> {
    progress(quiet, "resolving setup target");
    if setup.local {
        progress(
            quiet,
            "installing local binary into ~/.gumgum/bin and daemon service into ~/.gumgum/daemon",
        );
        install_local_user_service(quiet).await?;
        configure_host_dns(&setup.test_domain, quiet).await?;
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
    if !setup.local {
        configure_client_resolver(&setup.test_domain, &setup.host, quiet).await?;
    }
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

fn local_config_path() -> gumgum_core::Result<PathBuf> {
    Ok(gumgum_root()?.join("config.json"))
}

fn server_config_path(name: &str) -> gumgum_core::Result<PathBuf> {
    Ok(gumgum_root()?
        .join("servers")
        .join(sanitize_name(name))
        .join("config.json"))
}

fn load_config_map(
    path: &PathBuf,
) -> gumgum_core::Result<serde_json::Map<String, serde_json::Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let raw = fs::read_to_string(path).map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not read config")
            .likely_cause(source.to_string())
            .build()
    })?;
    serde_json::from_str(&raw).map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not parse config")
            .likely_cause(source.to_string())
            .build()
    })
}

fn save_config_map(
    path: &PathBuf,
    values: &serde_json::Map<String, serde_json::Value>,
) -> gumgum_core::Result<()> {
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
    fs::write(
        path,
        serde_json::to_string_pretty(values).expect("serialize config"),
    )
    .map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not write config")
            .likely_cause(source.to_string())
            .build()
    })
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
        .stdin(Stdio::inherit())
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

fn shell_escape_plain(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        .collect()
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
