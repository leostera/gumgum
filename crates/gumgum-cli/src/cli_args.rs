use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gumgum")]
#[command(about = "GumGum.dev local cloud control plane")]
#[command(version)]
pub struct Cli {
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
    Worker(WorkerArgs),
    Deploy(DeployArgs),
    Publish(PublishArgs),
    Env(EnvArgs),
    Info(InfoArgs),
    Rollback(RollbackArgs),
    Logs(LogsArgs),
    Events(EventsArgs),
    Graph(GraphArgs),
    Db(ObjectArgs),
    Kv(ObjectArgs),
    Bucket(BucketArgs),
    Queue(ObjectArgs),
    Secret(ObjectArgs),
    Setup(SetupArgs),
    Domain(DomainArgs),
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
    #[arg(
        value_name = "HOST",
        help = "Server host/name (compatibility positional)"
    )]
    pub(crate) target: Option<String>,
    #[arg(long, help = "Server host/name to ping")]
    pub(crate) host: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DeployEnv {
    Preview,
    Release,
}

#[allow(dead_code)]
impl DeployEnv {
    pub(crate) fn is_release(self) -> bool {
        self == Self::Release
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct DeployArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, value_enum, default_value = "preview")]
    pub(crate) env: DeployEnv,
    #[arg(long, help = "Delete the desired deployment for this worker")]
    pub(crate) delete: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PublishArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) target: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, help = "Public domain to plan instead of manifest/default route")]
    pub(crate) public_domain: Option<String>,
    #[arg(
        long,
        default_value = "byo",
        help = "Tunnel/provider surface to use in the plan"
    )]
    pub(crate) tunnel: String,
}

#[derive(Debug, Args)]
pub(crate) struct EnvArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) host: Option<String>,
    #[arg(long, help = "Only print environment for this project/workspace")]
    pub(crate) project: Option<String>,
    #[arg(long, help = "Only print environment for this worker")]
    pub(crate) worker: Option<String>,
    #[arg(long, help = "Prefix env vars with project and worker names")]
    pub(crate) qualified: bool,
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
    #[arg(
        long,
        help = "Delete a stale deployment revision id without changing containers"
    )]
    pub(crate) delete_revision_id: Option<i64>,
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
    List(ListObjectArgs),
    Create(CreateObjectArgs),
    Delete(DeleteObjectArgs),
    Bind(BindObjectArgs),
    Unbind(UnbindObjectArgs),
}

#[derive(Debug, Args)]
pub(crate) struct BucketArgs {
    #[command(subcommand)]
    pub(crate) command: BucketCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum BucketCommand {
    List(ListObjectArgs),
    Create(CreateObjectArgs),
    Delete(DeleteObjectArgs),
    Bind(BindObjectArgs),
    Unbind(UnbindObjectArgs),
    Ls(BucketPathArgs),
    Get(BucketPathArgs),
    Rm(BucketPathArgs),
    Cp(BucketCopyArgs),
    Sync(BucketCopyArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ListObjectArgs {
    #[arg(long)]
    pub(crate) host: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct BucketPathArgs {
    pub(crate) bucket: String,
    pub(crate) path: Option<String>,
    #[arg(long)]
    pub(crate) host: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct BucketCopyArgs {
    pub(crate) source: String,
    pub(crate) destination: String,
    #[arg(long)]
    pub(crate) host: Option<String>,
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
    #[arg(long, help = "Filter events by kind: mutation or reconciliation")]
    pub(crate) kind: Option<String>,
    #[arg(
        long,
        help = "Group events by operation id; with --json this returns a grouped report object"
    )]
    pub(crate) grouped: bool,
}

#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) domain: Option<String>,
    #[arg(long)]
    pub(crate) namespace: Option<String>,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args)]
pub(crate) struct WorkerArgs {
    #[command(subcommand)]
    pub(crate) command: WorkerCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum WorkerCommand {
    Create(WorkerCreateArgs),
    List(WorkerListArgs),
    Delete(WorkerDeleteArgs),
}

#[derive(Debug, Args)]
pub(crate) struct WorkerCreateArgs {
    pub(crate) name: String,
    #[arg(long, default_value_t = 3000)]
    pub(crate) port: u16,
    #[arg(long)]
    pub(crate) namespace: Option<String>,
    #[arg(long = "zone")]
    pub(crate) zones: Vec<String>,
    #[arg(
        long,
        help = "Create under this directory instead of the workspace root"
    )]
    pub(crate) dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args)]
pub(crate) struct WorkerListArgs {
    #[arg(default_value = "gumgum.toml")]
    pub(crate) workspace: PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct WorkerDeleteArgs {
    pub(crate) name: String,
    #[arg(default_value = "gumgum.toml")]
    pub(crate) workspace: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum IngressArg {
    Direct,
    Cloudflare,
}

impl From<IngressArg> for gumgum_core::IngressMode {
    fn from(value: IngressArg) -> Self {
        match value {
            IngressArg::Direct => Self::Direct,
            IngressArg::Cloudflare => Self::Cloudflare,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct SetupArgs {
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) user: Option<String>,
    #[arg(long)]
    pub(crate) domain: Option<String>,
    #[arg(long, value_enum, default_value_t = IngressArg::Direct)]
    pub(crate) ingress: IngressArg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DomainProviderArg {
    Manual,
    Cloudflare,
}

impl From<DomainProviderArg> for gumgum_core::DomainProvider {
    fn from(value: DomainProviderArg) -> Self {
        match value {
            DomainProviderArg::Manual => Self::Manual,
            DomainProviderArg::Cloudflare => Self::Cloudflare,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct DomainArgs {
    #[command(subcommand)]
    pub(crate) command: DomainCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DomainCommand {
    Add(DomainAddArgs),
}

#[derive(Debug, Args)]
pub(crate) struct DomainAddArgs {
    pub(crate) name: String,
    #[arg(long, value_enum, default_value_t = DomainProviderArg::Manual)]
    pub(crate) provider: DomainProviderArg,
    #[arg(long)]
    pub(crate) server: Option<String>,
    #[arg(long, value_enum, default_value_t = IngressArg::Direct)]
    pub(crate) ingress: IngressArg,
}

#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub(crate) struct ServerCommand {
    #[arg(value_name = "NAME")]
    pub(crate) name: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Option<ServerSubcommand>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServerSubcommand {
    List,
    Add(ServerAddArgs),
    Rm(ServerRmArgs),
    Ping(PingArgs),
    Capabilities(ServerCapabilitiesArgs),
    Config(ServerConfigArgs),
    Upgrade(ServerUpgradeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServerAddArgs {
    pub(crate) host: String,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) user: Option<String>,
    #[arg(long)]
    pub(crate) domain: Option<String>,
    #[arg(long, value_enum, default_value_t = IngressArg::Direct)]
    pub(crate) ingress: IngressArg,
}

#[derive(Debug, Args)]
pub(crate) struct ServerRmArgs {
    pub(crate) host_or_name: String,
}

#[derive(Debug, Args)]
pub(crate) struct ServerCapabilitiesArgs {
    #[arg(
        value_name = "ACTION",
        help = "Capability action; use `list` in the new form"
    )]
    pub(crate) action: Option<String>,
    #[arg(long, help = "Server host/name to inspect")]
    pub(crate) host: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Fail if any comma-separated capabilities are missing"
    )]
    pub(crate) require: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ServerHostArgs {
    #[arg(long, help = "Server host/name")]
    pub(crate) host: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ServerConfigArgs {
    #[arg(long, help = "Server host/name to configure")]
    pub(crate) host: Option<String>,
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Debug, Args)]
pub(crate) struct ServerUpgradeArgs {
    #[arg(long, help = "Server host/name to upgrade")]
    pub(crate) host: Option<String>,
    #[arg(long)]
    pub(crate) user: Option<String>,
}

#[allow(dead_code)]
pub fn parse_cli_args_for_fuzz(input: &str) -> bool {
    let args = std::iter::once("gumgum").chain(input.split_whitespace());
    Cli::try_parse_from(args).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_workspace_only_and_env_supports_qualified() {
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "init",
                "--name",
                "visit-counter",
                "--domain",
                "example.com",
            ])
            .unwrap()
            .command,
            Command::Init(InitArgs {
                name: Some(ref name),
                domain: Some(ref domain),
                ..
            }) if name == "visit-counter" && domain == "example.com"
        ));
        assert!(Cli::try_parse_from(["gumgum", "init", "--kind", "worker"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "env", "--qualified", "--worker", "api"])
                .unwrap()
                .command,
            Command::Env(EnvArgs { qualified: true, worker: Some(ref worker), .. })
                if worker == "api"
        ));
    }

    #[test]
    fn logs_follow_is_allowed_without_single_worker_at_parse_layer() {
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "logs", "-f"])
                .unwrap()
                .command,
            Command::Logs(LogsArgs { follow: true, .. })
        ));
    }

    #[test]
    fn server_capabilities_command_supports_explicit_requirements() {
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "server",
                "starbase2",
                "capabilities",
                "--require",
                "gumgum:events,gumgum:bindings:delete,gumgum:objects:delete",
            ])
            .unwrap()
            .command,
            Command::Server(ServerCommand {
                name: Some(ref name),
                command: Some(ServerSubcommand::Capabilities(ServerCapabilitiesArgs { ref require, .. }))
            }) if name == "starbase2"
                && require == &vec![
                    "gumgum:events".to_owned(),
                    "gumgum:bindings:delete".to_owned(),
                    "gumgum:objects:delete".to_owned(),
                ]
        ));
    }

    #[test]
    fn server_add_and_rm_commands_use_uniform_shape() {
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "server",
                "add",
                "starbase2",
                "--name",
                "starbase2",
                "--domain",
                "leostera.dev",
            ])
            .unwrap()
            .command,
            Command::Server(ServerCommand {
                name: None,
                command: Some(ServerSubcommand::Add(ServerAddArgs {
                    ref host,
                    name: Some(ref name),
                    domain: Some(ref root_domain),
                    ..
                }))
            }) if host == "starbase2" && name == "starbase2" && root_domain == "leostera.dev"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "server", "rm", "starbase2"])
                .unwrap()
                .command,
            Command::Server(ServerCommand {
                name: None,
                command: Some(ServerSubcommand::Rm(ServerRmArgs { ref host_or_name }))
            }) if host_or_name == "starbase2"
        ));
    }

    #[test]
    fn server_capabilities_supports_new_list_host_form() {
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "server",
                "capabilities",
                "list",
                "--host",
                "starbase2",
                "--require",
                "gumgum:events",
            ])
            .unwrap()
            .command,
            Command::Server(ServerCommand {
                name: None,
                command: Some(ServerSubcommand::Capabilities(ServerCapabilitiesArgs {
                    action: Some(ref action),
                    host: Some(ref host),
                    ref require,
                }))
            }) if action == "list" && host == "starbase2" && require == &vec!["gumgum:events".to_owned()]
        ));
    }

    #[test]
    fn worker_command_grammar_covers_create_list_delete() {
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "worker", "create", "api", "--port", "8080"])
                .unwrap()
                .command,
            Command::Worker(WorkerArgs {
                command: WorkerCommand::Create(WorkerCreateArgs { ref name, port: 8080, .. })
            }) if name == "api"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "worker", "list"])
                .unwrap()
                .command,
            Command::Worker(WorkerArgs {
                command: WorkerCommand::List(WorkerListArgs { .. })
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "worker", "delete", "api"])
                .unwrap()
                .command,
            Command::Worker(WorkerArgs {
                command: WorkerCommand::Delete(WorkerDeleteArgs { ref name, .. })
            }) if name == "api"
        ));
    }

    #[test]
    fn rollback_command_supports_safe_revision_delete() {
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "rollback",
                "api/gumgum.toml",
                "--host",
                "starbase2",
                "--worker",
                "visit-counter-api",
                "--delete-revision-id",
                "8",
            ])
            .unwrap()
            .command,
            Command::Rollback(RollbackArgs {
                ref path,
                host: Some(ref host),
                worker: Some(ref worker),
                delete_revision_id: Some(8),
                ..
            }) if path == std::path::Path::new("api/gumgum.toml")
                && host == "starbase2"
                && worker == "visit-counter-api"
        ));
    }

    #[test]
    fn bucket_command_grammar_covers_create_bind_delete_unbind() {
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "bucket", "list"])
                .unwrap()
                .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::List(ListObjectArgs { .. })
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "bucket", "ls", "visit-requests", "raw/*.json"])
                .unwrap()
                .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Ls(BucketPathArgs { ref bucket, path: Some(ref path), .. })
            }) if bucket == "visit-requests" && path == "raw/*.json"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "bucket", "cp", "local.json", "visit-requests/raw/local.json"])
                .unwrap()
                .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Cp(BucketCopyArgs { ref source, ref destination, .. })
            }) if source == "local.json" && destination == "visit-requests/raw/local.json"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "bucket", "create", "visit-requests"])
                .unwrap()
                .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Create(CreateObjectArgs { ref name, .. })
            }) if name == "visit-requests"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "bucket",
                "bind",
                "visit-requests",
                "--to",
                "api",
                "--as",
                "VISIT_REQUESTS_BUCKET",
                "--access",
                "read-only",
            ])
            .unwrap()
            .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Bind(BindObjectArgs {
                    ref name,
                    to: Some(ref to),
                    ref binding,
                    ref access,
                    ..
                })
            }) if name == "visit-requests" && to == "api" && binding == "VISIT_REQUESTS_BUCKET" && access == "read-only"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "bucket", "delete", "visit-requests", "--preview"])
                .unwrap()
                .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Delete(DeleteObjectArgs { ref name, preview: true, .. })
            }) if name == "visit-requests"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "bucket",
                "unbind",
                "visit-requests",
                "--to",
                "worker",
                "--as",
                "VISIT_REQUESTS_BUCKET",
                "--preview",
            ])
            .unwrap()
            .command,
            Command::Bucket(BucketArgs {
                command: BucketCommand::Unbind(UnbindObjectArgs {
                    ref name,
                    to: Some(ref to),
                    ref binding,
                    preview: true,
                    ..
                })
            }) if name == "visit-requests" && to == "worker" && binding == "VISIT_REQUESTS_BUCKET"
        ));
    }

    #[test]
    fn object_delete_preview_grammar_covers_all_resource_kinds() {
        for (kind, expected) in [
            ("db", "visits"),
            ("kv", "user-counters"),
            ("bucket", "visit-requests"),
            ("queue", "visit-events"),
            ("secret", "stripe-api-key"),
        ] {
            let command = Cli::try_parse_from(["gumgum", kind, "delete", expected, "--preview"])
                .unwrap()
                .command;
            match (kind, command) {
                (
                    "bucket",
                    Command::Bucket(BucketArgs {
                        command:
                            BucketCommand::Delete(DeleteObjectArgs {
                                name,
                                preview: true,
                                ..
                            }),
                    }),
                ) => assert_eq!(name, expected),
                (
                    _,
                    Command::Db(ObjectArgs {
                        command:
                            ObjectCommand::Delete(DeleteObjectArgs {
                                name,
                                preview: true,
                                ..
                            }),
                    })
                    | Command::Kv(ObjectArgs {
                        command:
                            ObjectCommand::Delete(DeleteObjectArgs {
                                name,
                                preview: true,
                                ..
                            }),
                    })
                    | Command::Queue(ObjectArgs {
                        command:
                            ObjectCommand::Delete(DeleteObjectArgs {
                                name,
                                preview: true,
                                ..
                            }),
                    })
                    | Command::Secret(ObjectArgs {
                        command:
                            ObjectCommand::Delete(DeleteObjectArgs {
                                name,
                                preview: true,
                                ..
                            }),
                    }),
                ) => assert_eq!(name, expected),
                _ => panic!("unexpected parsed command for {kind}"),
            }
        }
    }

    #[test]
    fn bucket_object_transfer_commands_are_bucket_only() {
        for kind in ["db", "kv", "queue", "secret"] {
            assert!(Cli::try_parse_from(["gumgum", kind, "ls", "objects"]).is_err());
            assert!(Cli::try_parse_from(["gumgum", kind, "get", "objects", "key"]).is_err());
            assert!(Cli::try_parse_from(["gumgum", kind, "cp", "a", "b"]).is_err());
            assert!(Cli::try_parse_from(["gumgum", kind, "sync", "a", "b"]).is_err());
        }
    }

    #[test]
    fn queue_command_grammar_covers_create_bind_delete_unbind() {
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "queue", "create", "visit-events"])
                .unwrap()
                .command,
            Command::Queue(ObjectArgs {
                command: ObjectCommand::Create(CreateObjectArgs { ref name, .. })
            }) if name == "visit-events"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "queue",
                "bind",
                "visit-events",
                "--to",
                "api",
                "--as",
                "VISIT_EVENTS_QUEUE",
            ])
            .unwrap()
            .command,
            Command::Queue(ObjectArgs {
                command: ObjectCommand::Bind(BindObjectArgs {
                    ref name,
                    to: Some(ref to),
                    ref binding,
                    ref access,
                    ..
                })
            }) if name == "visit-events" && to == "api" && binding == "VISIT_EVENTS_QUEUE" && access == "read-write"
        ));
        assert!(matches!(
            Cli::try_parse_from(["gumgum", "queue", "delete", "visit-events", "--preview"])
                .unwrap()
                .command,
            Command::Queue(ObjectArgs {
                command: ObjectCommand::Delete(DeleteObjectArgs { ref name, preview: true, .. })
            }) if name == "visit-events"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "gumgum",
                "queue",
                "unbind",
                "visit-events",
                "--to",
                "worker",
                "--as",
                "VISIT_EVENTS_QUEUE",
                "--preview",
            ])
            .unwrap()
            .command,
            Command::Queue(ObjectArgs {
                command: ObjectCommand::Unbind(UnbindObjectArgs {
                    ref name,
                    to: Some(ref to),
                    ref binding,
                    preview: true,
                    ..
                })
            }) if name == "visit-events" && to == "worker" && binding == "VISIT_EVENTS_QUEUE"
        ));
    }
}
