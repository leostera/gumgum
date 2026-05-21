use crate::server_client::ServerClient;
use crate::{
    SchemaCommand, SchemaSubcommand, ServerCommand, ServerCredentialsSubcommand,
    ServerProvidersSubcommand, ServerSubcommand, ServerUpgradeArgs, StatusArgs, config_command,
    print_value, progress,
};
use gumgum_api::{PingReport, ProviderConfigureRequest, ProviderStatusReport, ServerListReport};
use gumgum_core::{
    ConfigStore, DaemonHealthClient, DaemonPingReport, DoctorCheck, DoctorReport, ErrorCode,
    GumgumError, GumgumInstaller, ServerRecord, SetupTarget, Subsystem, not_configured_status,
    validate_path,
};
use serde::Serialize;
use std::path::PathBuf;
use std::str::FromStr;

pub(crate) async fn status(args: StatusArgs, json: bool) -> gumgum_core::Result<()> {
    if let Some(host) = args.host {
        let report = ping_host(&host).await?;
        print_value(json, &report)
    } else if let Some(server) = ConfigStore::from_home_env()?.load_default_server()? {
        let report = ping_host(&server.host).await?;
        print_value(json, &report)
    } else {
        print_value(json, &not_configured_status())
    }
    Ok(())
}

pub(crate) fn doctor(json: bool) {
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
    print_value(json, &report)
}

pub(crate) fn version(json: bool) {
    print_value(json, &version_report());
}

pub(crate) async fn server(server: ServerCommand, json: bool) -> gumgum_core::Result<()> {
    match server.command {
        Some(ServerSubcommand::List) => {
            let report = ServerListReport {
                ok: true,
                servers: ConfigStore::from_home_env()?.load_servers()?,
            };
            print_value(json, &report)
        }
        Some(ServerSubcommand::Ping(args)) => {
            let report = ping_host(&args.host).await?;
            print_value(json, &report)
        }
        Some(ServerSubcommand::BootProviders) => {
            let name = required_server_name(server.name, "boot-providers")?;
            let server = find_server(&name)?;
            let report = ServerClient::new(server.host)
                .boot_default_providers()
                .await?;
            print_value(json, &report);
        }
        Some(ServerSubcommand::Config(args)) => {
            let name = required_server_name(server.name, "config")?;
            let report = config_command(Some(name), args.command)?;
            print_value(json, &report)
        }
        Some(ServerSubcommand::Credentials(args)) => {
            let name = required_server_name(server.name, "credentials")?;
            let server = find_server(&name)?;
            match args.command {
                ServerCredentialsSubcommand::Init => {
                    let report = ServerClient::new(server.host)
                        .init_minio_credentials()
                        .await?;
                    print_value(json, &report);
                }
            }
        }
        Some(ServerSubcommand::Providers(args)) => {
            let name = required_server_name(server.name, "providers")?;
            let server = find_server(&name)?;
            match args.command.unwrap_or(ServerProvidersSubcommand::Status) {
                ServerProvidersSubcommand::Status => {
                    let report = ServerClient::new(server.host).providers().await?;
                    if json {
                        print_value(true, &report);
                    } else {
                        print_provider_status_report(&report);
                    }
                }
                ServerProvidersSubcommand::Boot => {
                    let report = ServerClient::new(server.host)
                        .boot_default_providers()
                        .await?;
                    print_value(json, &report);
                }
                ServerProvidersSubcommand::Configure(args) => {
                    let capability = gumgum_core::Capability::from_str(&args.capability)
                        .unwrap_or(gumgum_core::Capability::Manual);
                    let report = ServerClient::new(server.host)
                        .configure_provider(&ProviderConfigureRequest {
                            capability,
                            kind: args.kind,
                            endpoint: args.endpoint,
                            vault: args.vault,
                        })
                        .await?;
                    print_value(json, &report);
                }
            }
        }
        Some(ServerSubcommand::Upgrade(args)) => {
            let name = required_server_name(server.name, "upgrade")?;
            let report = upgrade_server(&name, args, json).await?;
            print_value(json, &report)
        }
        None if server.name.as_deref() == Some("list") => {
            let report = ServerListReport {
                ok: true,
                servers: ConfigStore::from_home_env()?.load_servers()?,
            };
            print_value(json, &report)
        }
        None => {
            return Err(GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::InvalidArgs,
                "server command is required",
            )
            .next_command("gumgum server list")
            .next_command("gumgum server <name> upgrade")
            .build());
        }
    }
    Ok(())
}

pub(crate) fn schema(schema: SchemaCommand, json: bool) -> gumgum_core::Result<()> {
    match schema.command {
        SchemaSubcommand::Validate { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("gumgum.toml"));
            let report = validate_path(&path)?;
            print_value(json, &report)
        }
        SchemaSubcommand::Explain => {
            let explanation = SchemaExplanation {
                ok: true,
                schemas: vec!["workspace", "worker"],
                message: "v0 supports [workspace] and [worker] manifests".to_owned(),
            };
            print_value(json, &explanation)
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ServerUpgradeReport {
    ok: bool,
    name: String,
    host: String,
    root_domain: String,
    test_domain: String,
    health_url: String,
    actions: Vec<String>,
    message: String,
}

fn required_server_name(name: Option<String>, command: &str) -> gumgum_core::Result<String> {
    name.ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            format!("server name is required for {command}"),
        )
        .next_command(format!("gumgum server <name> {command}"))
        .build()
    })
}

fn find_server(name: &str) -> gumgum_core::Result<ServerRecord> {
    ConfigStore::from_home_env()?
        .load_servers()?
        .into_iter()
        .find(|server| server.name == name || server.host == name)
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                format!("unknown GumGum.dev server {name}"),
            )
            .next_command("gumgum server list")
            .build()
        })
}

async fn upgrade_server(
    name: &str,
    args: ServerUpgradeArgs,
    quiet: bool,
) -> gumgum_core::Result<ServerUpgradeReport> {
    let server = find_server(name)?;
    let setup = SetupTarget {
        name: server.name.clone(),
        host: server.host.clone(),
        user: args.user,
        root_domain: server.root_domain.clone(),
        test_domain: server.test_domain.clone(),
        local: false,
    };
    let target = setup.ssh_target();
    progress(
        quiet,
        format!("upgrading gumgum on {target} from published release"),
    );
    GumgumInstaller::run_remote_setup(&target, &setup, quiet).await?;
    progress(quiet, "checking upgraded gumgumd health");
    DaemonHealthClient::wait_for_ping(&setup.host).await?;
    Ok(ServerUpgradeReport {
        ok: true,
        name: server.name,
        host: server.host,
        root_domain: server.root_domain,
        test_domain: server.test_domain,
        health_url: server.health_url,
        actions: vec![
            "ssh into server".to_owned(),
            "run published GumGum.dev installer".to_owned(),
            "restart gumgumd via remote setup".to_owned(),
            "check gumgumd health".to_owned(),
        ],
        message: "server upgraded from published release".to_owned(),
    })
}

fn print_provider_status_report(report: &ProviderStatusReport) {
    println!("Providers ({}):", report.providers.len());
    for provider in &report.providers {
        println!(
            "{} {} container={} image={} port={} running={}",
            provider.capability,
            provider.provider,
            provider.container,
            provider.image,
            provider.port,
            provider.running
        );
    }
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

#[derive(Debug, Serialize)]
struct SchemaExplanation {
    ok: bool,
    schemas: Vec<&'static str>,
    message: String,
}
