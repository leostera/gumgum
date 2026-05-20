mod daemon_app;
mod deploy_executor;
mod graph_presenter;
mod presentation;
mod server_client;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use daemon_app::DaemonApp;
use deploy_executor::DeployExecutor;
use graph_presenter::GraphPresenter;
use gumgum_api::{
    BindingRequest, DeployApplyReport, DeployRequest, GraphEdge, GraphNode, ObjectReport,
    ObjectRequest, PingReport, ServerListReport, SetupPlan, SetupReport,
};
use gumgum_core::{
    Capability, ConfigStore, DaemonHealthClient, DaemonPingReport, DeploymentDescriptor,
    DoctorCheck, DoctorReport, ErrorCode, GumgumError, GumgumInstaller,
    InitManifestKind as CoreInitKind, ManifestKind, PlanGraph, ServerRecord, SetupOptions,
    SetupTarget, Subsystem, WorkerManifest, default_project_name, init_plan, load_worker_path,
    load_workspace_path, not_configured_status,
    run_setup_command_streaming as run_command_streaming, sanitize_name, setup_actions,
    validate_path,
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
            } else if let Some(server) = ConfigStore::from_home_env()?.load_default_server()? {
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
                    servers: ConfigStore::from_home_env()?.load_servers()?,
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
    let store = ConfigStore::from_home_env()?;
    let scope = server_name
        .as_ref()
        .map(|name| format!("server:{name}"))
        .unwrap_or_else(|| "local".to_owned());
    let mut values = match &server_name {
        Some(name) => store.load_server_config(name)?,
        None => store.load_local_config()?,
    };
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
            match &server_name {
                Some(name) => store.save_server_config(name, &values)?,
                None => store.save_local_config(&values)?,
            };
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
        None => ConfigStore::from_home_env()?.load_default_server()?,
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
    progress(
        quiet,
        format!(
            "configuring local resolver for {} -> {}",
            server.test_domain, server.host
        ),
    );
    GumgumInstaller::configure_client_resolver(&server.test_domain, &server.host, quiet).await?;
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
    let descriptor = DeploymentDescriptor::from_manifest(&path, manifest, server, prod);
    DeployReport {
        ok: true,
        dry_run,
        path: path.display().to_string(),
        worker: descriptor.worker,
        host: server.map(|server| server.host.clone()),
        build_context: descriptor.build_context,
        image: descriptor.image,
        container: descriptor.container,
        port: descriptor.port,
        routes: descriptor.routes,
        health_url: descriptor.health_url,
        plan: descriptor.plan,
        plan_graph: descriptor.plan_graph,
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
            ConfigStore::from_home_env()?
                .load_default_server()?
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
        None => ConfigStore::from_home_env()?
            .load_default_server()?
            .ok_or_else(|| {
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
            ConfigStore::from_home_env()?
                .load_default_server()?
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
        ConfigStore::from_home_env()
            .and_then(|store| store.load_default_server())
            .ok()
            .flatten()
            .map(|server| server.root_domain)
    });
    let namespace = args.namespace.unwrap_or_else(|| name.clone());
    let plan = init_plan(
        match args.kind {
            InitKind::Workspace => CoreInitKind::Workspace,
            InitKind::Worker => CoreInitKind::Worker,
        },
        &name,
        &namespace,
        args.port,
        &args.zones,
        root_domain.as_deref(),
    );

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
        files.extend(scaffold_example_files(&plan.scaffold_files, dry_run)?);
    }

    if !dry_run {
        fs::write(&path, plan.manifest).map_err(|source| {
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

fn scaffold_example_files(
    files: &[gumgum_core::ScaffoldFile],
    dry_run: bool,
) -> gumgum_core::Result<Vec<String>> {
    let paths = files.iter().map(|file| file.path.to_owned()).collect();
    if dry_run {
        return Ok(paths);
    }

    for file in files {
        write_if_missing(file.path, file.contents)?;
    }
    Ok(paths)
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

async fn resolve_setup(args: SetupArgs) -> gumgum_core::Result<SetupTarget> {
    GumgumInstaller::resolve_target(SetupOptions {
        host: args.host,
        name: args.name,
        user: args.user,
        root_domain: args.root_domain,
        test_domain: args.test_domain,
    })
    .await
}

async fn ping_host(host: &str) -> gumgum_core::Result<PingReport> {
    Ok(ping_report_from_core(DaemonHealthClient::ping(host).await?))
}

fn ping_report_from_core(report: DaemonPingReport) -> PingReport {
    PingReport {
        ok: report.ok,
        host: report.host,
        health_url: report.health_url,
        service_active: report.service_active,
        health: report.health,
    }
}

async fn wait_for_ping(host: &str) -> gumgum_core::Result<PingReport> {
    Ok(ping_report_from_core(
        DaemonHealthClient::wait_for_ping(host).await?,
    ))
}

async fn install_gumgumd(setup: SetupTarget, quiet: bool) -> gumgum_core::Result<SetupReport> {
    progress(quiet, "resolving setup target");
    if setup.local {
        progress(
            quiet,
            "installing local binary into ~/.gumgum/bin and daemon service into ~/.gumgum/daemon",
        );
        GumgumInstaller::install_local_user_service(quiet).await?;
        progress(
            quiet,
            format!("configuring host DNS for *.{}", setup.test_domain),
        );
        GumgumInstaller::configure_host_dns(&setup.test_domain, quiet).await?;
    } else {
        let target = setup.ssh_target();
        progress(quiet, format!("running remote bootstrap on {target}"));
        GumgumInstaller::run_remote_setup(&target, &setup, quiet).await?;
    }
    progress(quiet, "checking gumgumd health");
    wait_for_ping(&setup.host).await?;
    let health_url = format!("http://{}:7777/healthz", setup.host);
    ConfigStore::from_home_env()?.save_server(ServerRecord {
        name: setup.name.clone(),
        host: setup.host.clone(),
        root_domain: setup.root_domain.clone(),
        test_domain: setup.test_domain.clone(),
        health_url: health_url.clone(),
    })?;
    if !setup.local {
        progress(
            quiet,
            format!(
                "configuring local resolver for {} -> {}",
                setup.test_domain, setup.host
            ),
        );
        GumgumInstaller::configure_client_resolver(&setup.test_domain, &setup.host, quiet).await?;
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

#[derive(Debug, Serialize)]
struct SchemaExplanation {
    ok: bool,
    schemas: Vec<&'static str>,
    message: String,
}

#[cfg(test)]
mod tests {
    use gumgum_core::derive_test_domain;

    #[test]
    fn derives_test_domain_from_root_domain() {
        assert_eq!(derive_test_domain("leostera.dev"), "leostera.test");
    }

    #[test]
    fn formats_ssh_target() {
        let mut target = gumgum_core::SetupTarget {
            name: "starbase".to_owned(),
            host: "192.168.0.3".to_owned(),
            user: None,
            root_domain: "leostera.dev".to_owned(),
            test_domain: "leostera.test".to_owned(),
            local: false,
        };
        assert_eq!(target.ssh_target(), "192.168.0.3");
        target.user = Some("root".to_owned());
        assert_eq!(target.ssh_target(), "root@192.168.0.3");
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(super::sanitize_name("Starbase2.local"), "starbase2-local");
        assert_eq!(super::sanitize_name("192.168.0.3"), "192-168-0-3");
    }
}
