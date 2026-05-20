mod daemon_app;
mod deploy_executor;
mod deploy_plan;
mod graph_presenter;
mod presentation;
mod server_client;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use daemon_app::DaemonApp;
use deploy_executor::DeployExecutor;
use deploy_plan::DeployPlanner;
use graph_presenter::GraphPresenter;
use gumgum_api::{
    BindingRequest, DeployApplyReport, DeployRequest, GraphEdge, GraphNode, ObjectReport,
    ObjectRequest, PingReport, ServerListReport, ServerRecord, SetupPlan, SetupReport,
};
use gumgum_core::{
    Capability, DoctorCheck, DoctorReport, ErrorCode, GumgumError, ManifestKind, PlanGraph,
    Subsystem, WorkerManifest, load_worker_path, load_workspace_path, not_configured_status,
    setup_actions, validate_path,
};
use serde::Serialize;
use server_client::ServerClient;
use std::{fs, path::PathBuf, process::Stdio, time::Duration};
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
    Doctor,
    Version,
    Config(ConfigArgs),
    Init(InitArgs),
    Deploy(DeployArgs),
    Info(InfoArgs),
    Rollback(RollbackArgs),
    Logs(LogsArgs),
    Graph(GraphArgs),
    Db(ObjectArgs),
    Kv(ObjectArgs),
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
struct InfoArgs {
    #[arg(default_value = "gumgum.toml")]
    path: PathBuf,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    worker: Option<String>,
}

#[derive(Debug, Args)]
struct RollbackArgs {
    #[arg(default_value = "gumgum.toml")]
    path: PathBuf,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    worker: Option<String>,
}

#[derive(Debug, Args)]
struct GraphArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    worker: Option<String>,
    resource: Option<String>,
    #[command(subcommand)]
    command: Option<GraphCommand>,
}

#[derive(Debug, Subcommand)]
enum GraphCommand {
    Show,
    Affected { target: String },
}

#[derive(Debug, Args)]
struct ObjectArgs {
    #[command(subcommand)]
    command: ObjectCommand,
}

#[derive(Debug, Subcommand)]
enum ObjectCommand {
    Create(CreateObjectArgs),
    Bind(BindObjectArgs),
}

#[derive(Debug, Args)]
struct CreateObjectArgs {
    name: String,
    #[arg(long)]
    host: Option<String>,
    #[arg(long, default_value = "root")]
    namespace: String,
    #[arg(long)]
    root_domain: Option<String>,
}

#[derive(Debug, Args)]
struct BindObjectArgs {
    name: String,
    #[arg(long)]
    host: Option<String>,
    #[arg(long = "to")]
    to: Option<String>,
    #[arg(long = "as")]
    binding: String,
    #[arg(long, default_value = "read-write")]
    access: String,
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
    Ping(PingArgs),
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
            print_deploy_output(cli.json, &report);
        }
        Command::Info(args) => {
            info(args, cli.json).await?;
        }
        Command::Rollback(args) => {
            rollback(args, cli.json).await?;
        }
        Command::Logs(args) => {
            logs(args, cli.json).await?;
        }
        Command::Graph(args) => {
            graph(args, cli.json).await?;
        }
        Command::Db(args) => {
            object_command("db", args, cli.json).await?;
        }
        Command::Kv(args) => {
            object_command("kv", args, cli.json).await?;
        }
        Command::Setup(args) => {
            let resolved = resolve_setup(args).await?;
            if cli.dry_run {
                let plan = SetupPlan {
                    ok: true,
                    name: resolved.name,
                    host: resolved.host,
                    user: resolved.user,
                    root_domain: resolved.root_domain,
                    test_domain: resolved.test_domain,
                    actions: setup_actions(resolved.local),
                };
                print_value(cli.json, &plan)
            } else {
                let report = install_gumgumd(resolved, cli.json).await?;
                print_value(cli.json, &report)
            }
        }
        Command::Daemon => DaemonApp::new().run().await?,
        Command::Server(server) => match server.command {
            ServerSubcommand::List => {
                let report = ServerListReport {
                    ok: true,
                    servers: load_servers()?,
                };
                print_value(cli.json, &report)
            }
            ServerSubcommand::Ping(args) => {
                let report = ping_host(&args.host).await?;
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
pub(crate) struct DeployReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) path: String,
    pub(crate) worker: String,
    pub(crate) host: Option<String>,
    pub(crate) build_context: Option<String>,
    pub(crate) image: String,
    pub(crate) container: String,
    pub(crate) port: u16,
    pub(crate) routes: Vec<String>,
    pub(crate) health_url: Option<String>,
    pub(crate) plan: Vec<String>,
    pub(crate) plan_graph: PlanGraph,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceDeployReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) path: String,
    pub(crate) workspace: String,
    pub(crate) workers: Vec<DeployReport>,
    pub(crate) plan: Vec<String>,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum DeployOutput {
    Worker(DeployReport),
    Workspace(WorkspaceDeployReport),
}

async fn deploy(args: DeployArgs, dry_run: bool, quiet: bool) -> gumgum_core::Result<DeployOutput> {
    let kind = validate_path(&args.path)?.manifest_kind;
    let server = match args.host.clone() {
        Some(host) => Some(ServerRecord {
            name: sanitize_name(&host),
            host: host.clone(),
            root_domain: String::new(),
            test_domain: String::new(),
            health_url: format!("http://{host}:7777/healthz"),
        }),
        None => load_default_server()?,
    };
    match kind {
        ManifestKind::Worker => {
            let manifest = load_worker_path(&args.path)?;
            let report = deploy_one(
                args.path.clone(),
                &manifest,
                server,
                dry_run,
                args.prod,
                quiet,
            )
            .await?;
            Ok(DeployOutput::Worker(report))
        }
        ManifestKind::Workspace => {
            let workspace = load_workspace_path(&args.path)?;
            let root = args
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut workers = Vec::new();
            let mut plan = vec![format!("workspace {}", workspace.workspace.name)];
            for member in &workspace.workspace.members {
                let member_path = root.join(member).join("gumgum.toml");
                let manifest = load_worker_path(&member_path)?;
                let report = deploy_one(
                    member_path,
                    &manifest,
                    server.clone(),
                    dry_run,
                    args.prod,
                    quiet,
                )
                .await?;
                plan.extend(
                    report
                        .plan
                        .iter()
                        .map(|step| format!("{}: {step}", report.worker)),
                );
                workers.push(report);
            }
            Ok(DeployOutput::Workspace(WorkspaceDeployReport {
                ok: true,
                dry_run,
                path: args.path.display().to_string(),
                workspace: workspace.workspace.name,
                workers,
                plan,
                message: if dry_run {
                    "workspace deploy plan"
                } else {
                    "workspace deployed"
                }
                .to_owned(),
            }))
        }
    }
}

async fn deploy_one(
    path: PathBuf,
    manifest: &WorkerManifest,
    server: Option<ServerRecord>,
    dry_run: bool,
    prod: bool,
    quiet: bool,
) -> gumgum_core::Result<DeployReport> {
    let mut report = deploy_report(path, manifest, server.as_ref(), dry_run, prod);
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
    DeployExecutor::new(&server, quiet)
        .ensure_manifest_bindings(manifest)
        .await?;
    run_remote_deploy(&server, manifest, &report, quiet).await?;
    configure_client_resolver(&server.test_domain, &server.host, quiet).await?;
    report.ok = true;
    report.dry_run = false;
    report.message = format!("deployed {} to {}", report.worker, server.host);
    Ok(report)
}

fn deploy_report(
    path: PathBuf,
    manifest: &WorkerManifest,
    server: Option<&ServerRecord>,
    dry_run: bool,
    prod: bool,
) -> DeployReport {
    let worker = manifest.worker.name.clone();
    let namespace = manifest
        .project
        .as_ref()
        .map(|project| project.namespace.as_str())
        .unwrap_or("root");
    let domain_scope = server
        .map(|server| dns_scope(&server.root_domain))
        .unwrap_or_else(|| "local".to_owned());
    let revision = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let image = format!("127.0.0.1:55000/{domain_scope}/{namespace}/{worker}:{revision}");
    let container = format!(
        "gumgum-{}",
        sanitize_name(&format!("{domain_scope}-{namespace}-{worker}"))
    );
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
    let build_context = manifest.worker.build_context.as_ref().map(|context| {
        let context_path = PathBuf::from(context);
        if context_path.is_absolute() {
            context_path.display().to_string()
        } else {
            path.parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(context_path)
                .display()
                .to_string()
        }
    });
    DeployReport {
        ok: true,
        dry_run,
        path: path.display().to_string(),
        worker,
        host: server.map(|server| server.host.clone()),
        build_context,
        image,
        container,
        port: manifest.worker.port.unwrap_or(3000),
        routes,
        health_url,
        plan: DeployPlanner::new(manifest).plan_lines(),
        plan_graph: DeployPlanner::new(manifest).graph(),
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

#[derive(Debug, Serialize)]
struct InfoReport {
    ok: bool,
    worker: String,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    urls: Vec<String>,
    latest_image: Option<String>,
    message: String,
}

async fn info(args: InfoArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let target = format!("worker/{worker}");
    let affected = ServerClient::new(server.host).affected(&target).await?;
    let urls = affected
        .nodes
        .iter()
        .filter(|node| node.kind == "route")
        .map(|node| format!("http://{}", node.label))
        .collect::<Vec<_>>();
    let latest_image = affected
        .nodes
        .iter()
        .find(|node| node.kind == "image")
        .map(|node| node.label.clone());
    let report = InfoReport {
        ok: true,
        worker,
        nodes: affected.nodes,
        edges: affected.edges,
        urls,
        latest_image,
        message: "current project info".to_owned(),
    };
    if json {
        print_value(true, &report);
    } else {
        println!("Worker: {}", report.worker);
        for url in &report.urls {
            println!("URL: {url}");
        }
        if let Some(image) = &report.latest_image {
            println!("Image: {image}");
        }
    }
    Ok(())
}

async fn rollback(args: RollbackArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host).rollback(worker).await?;
    print_value(json, &report);
    Ok(())
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
    let scoped_target = args
        .resource
        .or_else(|| args.worker.map(|worker| format!("worker/{worker}")))
        .or_else(|| infer_graph_target_from_manifest().ok().flatten());
    match args.command.unwrap_or(GraphCommand::Show) {
        GraphCommand::Show => {
            if let Some(target) = scoped_target {
                graph_affected(&server, &normalize_graph_target(&target), json).await
            } else {
                graph_show(&server, json).await
            }
        }
        GraphCommand::Affected { target } => {
            graph_affected(&server, &normalize_graph_target(&target), json).await
        }
    }
}

fn infer_graph_target_from_manifest() -> gumgum_core::Result<Option<String>> {
    let path = PathBuf::from("gumgum.toml");
    if path.exists() {
        return Ok(Some(format!(
            "worker/{}",
            load_worker_path(&path)?.worker.name
        )));
    }
    Ok(None)
}

fn normalize_graph_target(target: &str) -> String {
    if target.contains('/') {
        target.to_owned()
    } else if target.contains('.') {
        format!("route/{target}")
    } else {
        format!("worker/{target}")
    }
}

async fn graph_show(server: &str, json: bool) -> gumgum_core::Result<()> {
    let report = ServerClient::new(server).graph().await?;
    if json {
        print_value(true, &report);
    } else {
        println!("{}", report.graph);
    }
    Ok(())
}

async fn graph_affected(server: &str, target: &str, json: bool) -> gumgum_core::Result<()> {
    let report = ServerClient::new(server).affected(target).await?;
    if json {
        print_value(true, &report);
    } else {
        println!("Affected by {}:", report.target);
        let presenter = GraphPresenter::new();
        for node in report.nodes {
            println!("  {}", presenter.describe_node(&node));
        }
    }
    Ok(())
}

async fn object_command(kind: &str, args: ObjectArgs, json: bool) -> gumgum_core::Result<()> {
    let capability = capability_from_cli_kind(kind);
    match args.command {
        ObjectCommand::Create(args) => create_object(capability, args, json).await,
        ObjectCommand::Bind(args) => bind_object(capability, args, json).await,
    }
}

fn capability_from_cli_kind(kind: &str) -> Capability {
    match kind {
        "db" => Capability::Db,
        "kv" => Capability::Kv,
        _ => Capability::Manual,
    }
}

async fn create_object(
    capability: Capability,
    args: CreateObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let root_domain = args
        .root_domain
        .unwrap_or_else(|| server.root_domain.clone());
    let request = ObjectRequest {
        capability,
        name: args.name,
        namespace: args.namespace,
        root_domain,
    };
    let report: ObjectReport = ServerClient::new(server.host)
        .create_object(&request)
        .await?;
    if json {
        print_value(true, &report);
    } else {
        print_object_report(&report);
    }
    Ok(())
}

fn print_object_report(report: &ObjectReport) {
    presentation::Presenter::new().object_report(report);
}

async fn bind_object(
    capability: Capability,
    args: BindObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let worker = match args.to {
        Some(worker) => worker,
        None => load_worker_path(&PathBuf::from("gumgum.toml"))?.worker.name,
    };
    let request = BindingRequest {
        capability,
        object_name: args.name,
        worker,
        binding: args.binding,
        access: args.access,
    };
    let report = ServerClient::new(server.host).bind_object(&request).await?;
    if json {
        print_value(true, &report);
    } else {
        println!(
            "bound {} to {} as {}",
            report.object, report.worker, report.binding
        );
    }
    Ok(())
}

fn resolve_server(host: Option<String>) -> gumgum_core::Result<ServerRecord> {
    match host {
        Some(host) => Ok(ServerRecord {
            name: sanitize_name(&host),
            host: host.clone(),
            root_domain: String::new(),
            test_domain: String::new(),
            health_url: format!("http://{host}:7777/healthz"),
        }),
        None => load_default_server()?.ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "no GumGum.dev server configured",
            )
            .next_command("gumgum setup <host> --root-domain <domain>")
            .build()
        }),
    }
}

async fn logs(args: LogsArgs, quiet: bool) -> gumgum_core::Result<()> {
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
    if args.follow && quiet {
        return Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::InvalidArgs,
            "gumgum logs -f does not support --json yet",
        )
        .next_command("gumgum logs --json")
        .build());
    }
    if args.follow {
        let mut seen = String::new();
        loop {
            let report = ServerClient::new(&server)
                .logs(&container, args.tail)
                .await?;
            if let Some(delta) = report.logs.strip_prefix(&seen) {
                print!("{delta}");
            } else {
                print!("{}", report.logs);
            }
            seen = report.logs;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
        return Ok(());
    }
    let report = ServerClient::new(&server)
        .logs(&container, args.tail)
        .await?;
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
    let context = report
        .build_context
        .as_deref()
        .unwrap_or_else(|| manifest.worker.build_context.as_deref().unwrap_or("."));
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
    ServerClient::new(host).deploy(request).await
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

pub(crate) async fn run_command_streaming(
    cmd: &mut TokioCommand,
    quiet: bool,
) -> gumgum_core::Result<()> {
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

fn print_deploy_output(json: bool, output: &DeployOutput) {
    if json {
        print_value(true, output);
    } else {
        presentation::Presenter::new().deploy_output(output);
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

fn dns_scope(root_domain: &str) -> String {
    root_domain
        .trim_end_matches('.')
        .split('.')
        .rev()
        .collect::<Vec<_>>()
        .join(".")
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
