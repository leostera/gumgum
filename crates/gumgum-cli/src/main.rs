mod cli_output;
mod config_command;
mod daemon_app;
mod deploy_command;
mod deploy_executor;
mod graph_command;
mod graph_presenter;
mod init_command;
mod logs_command;
mod object_command;
mod presentation;
mod project_command;
mod server_client;
mod setup_command;
mod system_command;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
pub(crate) use cli_output::{print_error, print_value, progress};
use config_command::config_command;
use daemon_app::DaemonApp;
use deploy_command::{deploy, print_deploy_output};
use graph_command::graph;
use gumgum_api::SetupPlan;
use gumgum_core::{
    ConfigStore, ErrorCode, GumgumError, ServerRecord, Subsystem, sanitize_name, setup_actions,
};
use init_command::init_manifest;
use logs_command::logs;
use object_command::object_command;
use project_command::{info, rollback};
use setup_command::{install_gumgumd, resolve_setup};
use std::path::PathBuf;
use system_command::{doctor, schema, server, status, version};

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
pub(crate) struct StatusArgs {
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    user: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PingArgs {
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
pub(crate) enum ConfigSubcommand {
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
pub(crate) struct ServerCommand {
    #[command(subcommand)]
    command: ServerSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServerSubcommand {
    List,
    Ping(PingArgs),
    Config(ServerConfigArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServerConfigArgs {
    name: String,
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Args)]
pub(crate) struct SchemaCommand {
    #[command(subcommand)]
    command: SchemaSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SchemaSubcommand {
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
        Command::Status(args) => status(args, cli.json).await?,
        Command::Doctor => doctor(cli.json),
        Command::Version => version(cli.json),
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
        Command::Server(args) => server(args, cli.json).await?,
        Command::Schema(args) => schema(args, cli.json)?,
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
