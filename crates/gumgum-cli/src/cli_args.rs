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
    Version,
    Config(ConfigArgs),
    Init(InitArgs),
    Deploy(DeployArgs),
    Env(EnvArgs),
    Info(InfoArgs),
    Rollback(RollbackArgs),
    Logs(LogsArgs),
    Events(EventsArgs),
    Graph(GraphArgs),
    Db(ObjectArgs),
    Kv(ObjectArgs),
    Bucket(ObjectArgs),
    Queue(ObjectArgs),
    Secret(ObjectArgs),
    Setup(SetupArgs),
    Server(ServerCommand),
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
    #[arg(long, help = "Delete the desired deployment for this worker")]
    pub(crate) delete: bool,
}

#[derive(Debug, Args)]
pub(crate) struct EnvArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) worker: Option<String>,
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
    Delete(DeleteObjectArgs),
    Bind(BindObjectArgs),
    Unbind(UnbindObjectArgs),
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
    #[arg(
        long,
        help = "Use this password for credential-backed objects such as db"
    )]
    pub(crate) password: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct DeleteObjectArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, default_value = "root")]
    pub(crate) namespace: String,
    #[arg(long)]
    pub(crate) root_domain: Option<String>,
    #[arg(long)]
    pub(crate) preview: bool,
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
pub(crate) struct UnbindObjectArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long = "to")]
    pub(crate) to: Option<String>,
    #[arg(long = "as")]
    pub(crate) binding: String,
    #[arg(long)]
    pub(crate) preview: bool,
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
pub(crate) struct EventsArgs {
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: u32,
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
    BootProviders,
    Config(ServerConfigArgs),
    Credentials(ServerCredentialsArgs),
    Providers(ServerProvidersArgs),
    Upgrade(ServerUpgradeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServerConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Debug, Args)]
pub(crate) struct ServerCredentialsArgs {
    #[command(subcommand)]
    pub(crate) command: ServerCredentialsSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServerCredentialsSubcommand {
    Init,
}

#[derive(Debug, Args)]
pub(crate) struct ServerProvidersArgs {
    #[command(subcommand)]
    pub(crate) command: Option<ServerProvidersSubcommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServerProvidersSubcommand {
    Boot,
    Configure(ServerProviderConfigureArgs),
    Status,
}

#[derive(Debug, Args)]
pub(crate) struct ServerProviderConfigureArgs {
    pub(crate) capability: String,
    pub(crate) kind: String,
    #[arg(long)]
    pub(crate) endpoint: Option<String>,
    #[arg(long)]
    pub(crate) vault: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ServerUpgradeArgs {
    #[arg(long)]
    pub(crate) user: Option<String>,
}
