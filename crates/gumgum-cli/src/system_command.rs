use crate::server_client::ServerClient;
use crate::{
    ServerCommand, ServerCredentialsSubcommand, ServerProvidersSubcommand, ServerSubcommand,
    ServerUpgradeArgs, StatusArgs, config_command, print_value, progress,
};
use gumgum_api::{
    GraphReport, PingReport, ProviderConfigureRequest, ProviderStatusReport, ServerListReport,
};
use gumgum_core::{
    ConfigStore, DaemonHealthClient, DaemonPingReport, ErrorCode, GumgumError, GumgumInstaller,
    ServerRecord, SetupTarget, Subsystem, not_configured_status,
};
use serde::Serialize;
use std::str::FromStr;

pub(crate) async fn status(args: StatusArgs, json: bool) -> gumgum_core::Result<()> {
    if let Some(host) = args.host {
        let report = ping_host(&host).await?;
        if json {
            print_value(true, &report)
        } else {
            print_status_summary(&host, &report).await?;
        }
    } else if let Some(server) = ConfigStore::from_home_env()?.load_default_server()? {
        let report = ping_host(&server.host).await?;
        if json {
            print_value(true, &report)
        } else {
            print_status_summary(&server.host, &report).await?;
        }
    } else {
        print_value(json, &not_configured_status())
    }
    Ok(())
}

pub(crate) fn version(json: bool) {
    print_value(json, &version_report());
}

pub(crate) async fn server(
    server: ServerCommand,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    match server.command {
        Some(ServerSubcommand::List) => {
            let report = ServerListReport {
                ok: true,
                servers: ConfigStore::from_home_env()?.load_servers()?,
            };
            if json {
                print_value(true, &report);
            } else {
                print_server_list(&report.servers);
            }
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
            let report = upgrade_server(&name, args, json, dry_run).await?;
            print_value(json, &report)
        }
        None if server.name.as_deref() == Some("list") => {
            let report = ServerListReport {
                ok: true,
                servers: ConfigStore::from_home_env()?.load_servers()?,
            };
            if json {
                print_value(true, &report);
            } else {
                print_server_list(&report.servers);
            }
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
    dry_run: bool,
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
    let actions = server_upgrade_actions(dry_run);
    if dry_run {
        return Ok(ServerUpgradeReport {
            ok: true,
            name: server.name,
            host: server.host,
            root_domain: server.root_domain,
            test_domain: server.test_domain,
            health_url: server.health_url,
            actions,
            message: "server upgrade preview; no remote changes".to_owned(),
        });
    }
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
        actions,
        message: "server upgraded from published release".to_owned(),
    })
}

fn server_upgrade_actions(dry_run: bool) -> Vec<String> {
    let mut actions = vec![
        "ssh into server".to_owned(),
        "run published GumGum.dev installer".to_owned(),
        "restart gumgumd via remote setup".to_owned(),
        "check gumgumd health".to_owned(),
    ];
    if dry_run {
        actions.insert(0, "preview only; no ssh command will run".to_owned());
    }
    actions
}

fn print_server_list(servers: &[ServerRecord]) {
    if servers.is_empty() {
        println!("No GumGum.dev servers configured.");
        println!("Run: gumgum setup <host> --root-domain <domain>");
        return;
    }
    println!("{:<18} {:<16} {:<20} HEALTH", "NAME", "HOST", "ROOT DOMAIN");
    for server in servers {
        println!(
            "{:<18} {:<16} {:<20} {}",
            server.name, server.host, server.root_domain, server.health_url
        );
    }
}

fn print_provider_status_report(report: &ProviderStatusReport) {
    for line in provider_status_lines(report) {
        println!("{line}");
    }
}

async fn print_status_summary(host: &str, ping: &PingReport) -> gumgum_core::Result<()> {
    let providers = ServerClient::new(host).providers().await.ok();
    let graph = ServerClient::new(host).graph().await.ok();
    for line in status_summary_lines(ping, providers.as_ref(), graph.as_ref()) {
        println!("{line}");
    }
    Ok(())
}

fn provider_status_lines(report: &ProviderStatusReport) -> Vec<String> {
    let running = report
        .providers
        .iter()
        .filter(|provider| provider.running)
        .count();
    let mut lines = vec![format!(
        "Providers: {running}/{} running",
        report.providers.len()
    )];
    for provider in &report.providers {
        lines.push(format!(
            "  - {} {} container={} image={} port={} running={}",
            provider.capability,
            provider.provider,
            provider.container,
            provider.image,
            provider.port,
            provider.running
        ));
    }
    lines
}

fn status_summary_lines(
    ping: &PingReport,
    providers: Option<&ProviderStatusReport>,
    graph: Option<&GraphReport>,
) -> Vec<String> {
    let mut lines = vec![format!(
        "gumgumd: {} ({})",
        if ping.ok { "healthy" } else { "unhealthy" },
        ping.health_url
    )];
    if let Some(service_active) = ping.service_active {
        lines.push(format!("service: active={service_active}"));
    }
    if let Some(providers) = providers {
        lines.extend(provider_status_lines(providers));
        if providers.providers.iter().any(|provider| !provider.running) {
            lines.push(
                "provider warning: one or more providers are down; gumgumd is still responding"
                    .to_owned(),
            );
        }
    } else {
        lines.push("Providers: unavailable (gumgumd health still responded)".to_owned());
    }
    if let Some(graph) = graph {
        let workers = graph
            .nodes
            .iter()
            .filter(|node| node.kind == "worker")
            .count();
        let routes = graph
            .nodes
            .iter()
            .filter(|node| node.kind == "route")
            .count();
        let objects = graph
            .nodes
            .iter()
            .filter(|node| node.kind == "object")
            .count();
        lines.push(format!(
            "Desired graph: workers={workers} routes={routes} objects={objects}"
        ));
        for route in graph.nodes.iter().filter(|node| node.kind == "route") {
            lines.push(format!("  - route {}", route.label));
        }
    } else {
        lines.push("Desired graph: unavailable".to_owned());
    }
    lines
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

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{Capability, GraphEdge, GraphNode, ProviderStatus};

    #[test]
    fn server_upgrade_actions_explain_dry_run_safety() {
        assert_eq!(
            server_upgrade_actions(true)[0],
            "preview only; no ssh command will run"
        );
        assert!(
            !server_upgrade_actions(false)
                .iter()
                .any(|action| action.contains("preview only"))
        );
    }

    #[test]
    fn status_summary_includes_provider_route_and_down_warning() {
        let ping = PingReport {
            ok: true,
            host: "starbase2".to_owned(),
            health_url: "http://starbase2:7777/healthz".to_owned(),
            service_active: Some(true),
            health: serde_json::json!({"ok": true}),
        };
        let providers = ProviderStatusReport {
            ok: true,
            providers: vec![ProviderStatus {
                capability: Capability::Db,
                provider: "postgres.main".to_owned(),
                container: "gumgum-postgres".to_owned(),
                image: "postgres:16".to_owned(),
                port: 5432,
                running: false,
            }],
            message: "provider statuses".to_owned(),
        };
        let graph = GraphReport {
            ok: true,
            format: "json".to_owned(),
            graph: String::new(),
            nodes: vec![
                GraphNode::new("worker/api", "worker", "api"),
                GraphNode::new(
                    "route/api.visit-counter.leostera.test",
                    "route",
                    "api.visit-counter.leostera.test",
                ),
                GraphNode::new("object/db/visits", "object", "visits"),
            ],
            edges: Vec::<GraphEdge>::new(),
        };

        let lines = status_summary_lines(&ping, Some(&providers), Some(&graph));

        assert!(lines.contains(&"gumgumd: healthy (http://starbase2:7777/healthz)".to_owned()));
        assert!(lines.contains(&"Providers: 0/1 running".to_owned()));
        assert!(lines.iter().any(|line| line.contains("provider warning")));
        assert!(lines.contains(&"Desired graph: workers=1 routes=1 objects=1".to_owned()));
        assert!(lines.contains(&"  - route api.visit-counter.leostera.test".to_owned()));
    }
}
