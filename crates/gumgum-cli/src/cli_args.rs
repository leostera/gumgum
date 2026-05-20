use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gumgum")]
#[command(about = "GumGum.dev local cloud control plane")]
#[command(version)]
pub(crate) struct Cli {
    #[arg(long, global = true, help = "Emit stable JSON output")]
    pub(crate) json: bool,
    #[arg(long, global = true, help = "Plan without mutating state")]
    pub(crate) dry_run: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
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
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) user: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct PingArgs {
    pub(crate) host: String,
    #[arg(long)]
    pub(crate) user: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigSubcommand {
    List,
    Get { key: String },
    Set { key: String, value: String },
}

#[derive(Debug, Args)]
pub(crate) struct DeployArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) prod: bool,
}

#[derive(Debug, Args)]
pub(crate) struct InfoArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) worker: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct RollbackArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) worker: Option<String>,
    #[arg(
        long,
        help = "Show the previous deployment that would be restored without applying it"
    )]
    pub(crate) preview: bool,
    #[arg(
        long,
        help = "List previous deployment revisions instead of applying rollback"
    )]
    pub(crate) revisions: bool,
    #[arg(long, help = "Rollback or preview a specific deployment revision id")]
    pub(crate) revision_id: Option<i64>,
    #[arg(long, default_value_t = 10)]
    pub(crate) limit: u32,
}

#[derive(Debug, Args)]
pub(crate) struct GraphArgs {
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) project: Option<String>,
    #[arg(long)]
    pub(crate) worker: Option<String>,
    pub(crate) resource: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Option<GraphCommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum GraphCommand {
    Show,
    Affected { target: String },
}

#[derive(Debug, Args)]
pub(crate) struct ObjectArgs {
    #[command(subcommand)]
    pub(crate) command: ObjectCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ObjectCommand {
    Create(CreateObjectArgs),
    Bind(BindObjectArgs),
}

#[derive(Debug, Args)]
pub(crate) struct CreateObjectArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, default_value = "root")]
    pub(crate) namespace: String,
    #[arg(long)]
    pub(crate) root_domain: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct BindObjectArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long = "to")]
    pub(crate) to: Option<String>,
    #[arg(long = "as")]
    pub(crate) binding: String,
    #[arg(long, default_value = "read-write")]
    pub(crate) access: String,
}

#[derive(Debug, Args)]
pub(crate) struct LogsArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long, short)]
    pub(crate) follow: bool,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub(crate) tail: u32,
}

#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long, default_value = "worker")]
    pub(crate) kind: InitKind,
    #[arg(long, default_value_t = 3000)]
    pub(crate) port: u16,
    #[arg(long)]
    pub(crate) root_domain: Option<String>,
    #[arg(long)]
    pub(crate) namespace: Option<String>,
    #[arg(long = "zone")]
    pub(crate) zones: Vec<String>,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(crate) enum InitKind {
    Workspace,
    Worker,
}

#[derive(Debug, Args)]
pub(crate) struct SetupArgs {
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) user: Option<String>,
    #[arg(long)]
    pub(crate) root_domain: Option<String>,
    #[arg(long)]
    pub(crate) test_domain: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ServerCommand {
    #[arg(value_name = "NAME")]
    pub(crate) name: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Option<ServerSubcommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServerSubcommand {
    List,
    Ping(PingArgs),
    Config(ServerConfigArgs),
    Upgrade(ServerUpgradeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServerConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Debug, Args)]
pub(crate) struct ServerUpgradeArgs {
    #[arg(long)]
    pub(crate) user: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct SchemaCommand {
    #[command(subcommand)]
    pub(crate) command: SchemaSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SchemaSubcommand {
    Validate { path: Option<PathBuf> },
    Explain,
}
