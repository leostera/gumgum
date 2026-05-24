use crate::{
    ServerCommand, ServerSubcommand, ServerUpgradeArgs, StatusArgs, config_command,
    print_config_report, print_value, progress,
};
use crate::{
    domain_command::authorize_cloudflare_zone, presentation::action_texts,
    server_client::ServerClient,
};
use gumgum_api::{
    DaemonVersionReport, DomainAddRequest, GraphReport, PingReport, ProviderStatusReport,
    ServerListReport,
};
use gumgum_core::{
    ConfigStore, DaemonHealthClient, DaemonPingReport, ErrorCode, GumgumError, GumgumInstaller,
    ServerRecord, SetupOptions, SetupTarget, Subsystem, not_configured_status, setup_actions,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

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
        Some(ServerSubcommand::Add(args)) => {
            let report = add_server(args, dry_run, json).await?;
            if json {
                print_value(true, &report);
            } else {
                print_server_mutation_report(&report);
            }
        }
        Some(ServerSubcommand::Rm(args)) => {
            let report = remove_server(&args.host_or_name)?;
            if json {
                print_value(true, &report);
            } else {
                print_server_mutation_report(&report);
            }
        }
        Some(ServerSubcommand::Ping(args)) => {
            let host = ping_target(&args)?;
            let report = ping_host(&host).await?;
            if json {
                print_value(true, &report)
            } else {
                print_ping_report(&report);
            }
        }
        Some(ServerSubcommand::Capabilities(args)) => {
            let name = server_capabilities_target(server.name, &args)?;
            let server = find_server(&name)?;
            let report = ServerClient::new(server.host).version().await?;
            let required = required_capabilities_from_args(&args.require);
            if !required.is_empty() {
                require_capabilities(&name, &report, &required)?;
            }
            if json {
                print_value(true, &report);
            } else {
                for line in capability_lines(&report, &required) {
                    println!("{line}");
                }
            }
        }
        Some(ServerSubcommand::Config(args)) => {
            let name = server_command_target(server.name, args.host, "config")?;
            let report = config_command(Some(name), args.command)?;
            if json {
                print_value(true, &report);
            } else {
                print_config_report(&report);
            }
        }
        Some(ServerSubcommand::Upgrade(args)) => {
            let name = server_command_target(server.name, args.host.clone(), "upgrade")?;
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

#[derive(Debug, Serialize)]
struct ServerMutationReport {
    ok: bool,
    dry_run: bool,
    action: String,
    server: Option<ServerRecord>,
    actions: Vec<String>,
    message: String,
}

async fn add_server(
    args: crate::ServerAddArgs,
    dry_run: bool,
    quiet: bool,
) -> gumgum_core::Result<ServerMutationReport> {
    let setup = GumgumInstaller::resolve_target(SetupOptions {
        host: Some(args.host),
        name: args.name,
        user: args.user,
        root_domain: args.domain,
        test_domain: None,
        ingress: Some(args.ingress.into()),
    })
    .await?;
    reject_accidental_domain_replacement(&setup)?;
    let server = ServerRecord {
        name: setup.name.clone(),
        host: setup.host.clone(),
        root_domain: setup.root_domain.clone(),
        test_domain: setup.test_domain.clone(),
        health_url: format!("http://{}:7777/healthz", setup.host),
    };
    let actions = server_add_actions(&setup, dry_run);
    if dry_run {
        return Ok(ServerMutationReport {
            ok: true,
            dry_run: true,
            action: "add".to_owned(),
            message: "server add setup preview; no install, config, or providers changed"
                .to_owned(),
            server: Some(server),
            actions,
        });
    }

    if setup.local {
        progress(
            quiet,
            "installing local binary into ~/.gumgum/bin and daemon service into ~/.gumgum/daemon",
        );
        GumgumInstaller::install_local_user_service(quiet).await?;
    } else {
        let target = setup.ssh_target();
        progress(quiet, format!("running remote bootstrap on {target}"));
        GumgumInstaller::run_remote_setup(&target, &setup, quiet).await?;
    }
    progress(quiet, "checking gumgumd health");
    DaemonHealthClient::wait_for_ping(&setup.host).await?;
    ConfigStore::from_home_env()?.save_server(server.clone())?;
    progress(quiet, "initializing built-in providers");
    let client = ServerClient::new(setup.host.clone());
    let provider_report = client.boot_default_providers().await?;
    let mut actions = server_add_actions(&setup, false);
    actions.extend(action_texts(&provider_report.actions));
    if setup.ingress == gumgum_core::IngressMode::Cloudflare {
        progress(
            quiet,
            format!("authorizing Cloudflare for {}", setup.root_domain),
        );
        let grant = authorize_cloudflare_zone(&setup.root_domain)?;
        let domain_report = client
            .add_domain(&DomainAddRequest {
                name: setup.root_domain.clone(),
                provider: gumgum_core::DomainProvider::Cloudflare,
                ingress: setup.ingress,
                cloudflare_grant: Some(grant),
            })
            .await?;
        actions.extend(action_texts(&domain_report.actions));
    }
    Ok(ServerMutationReport {
        ok: true,
        dry_run: false,
        action: "add".to_owned(),
        message: format!("server {} setup complete", server.name),
        server: Some(server),
        actions,
    })
}

fn reject_accidental_domain_replacement(setup: &SetupTarget) -> gumgum_core::Result<()> {
    let existing = ConfigStore::from_home_env()?
        .load_servers()?
        .into_iter()
        .find(|server| server.name == setup.name || server.host == setup.host);
    if let Some(existing) = existing {
        if !existing.root_domain.is_empty()
            && !setup.root_domain.is_empty()
            && existing.root_domain != setup.root_domain
        {
            return Err(GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                format!(
                    "server {} already owns root domain {}",
                    existing.name, existing.root_domain
                ),
            )
            .likely_cause(format!(
                "refusing to replace it with {} during server add",
                setup.root_domain
            ))
            .next_command("gumgum server list")
            .build());
        }
    }
    Ok(())
}

fn server_add_actions(setup: &SetupTarget, dry_run: bool) -> Vec<String> {
    let mut actions = Vec::new();
    if dry_run {
        actions.push("preview only; no install, config, or provider changes".to_owned());
    }
    actions.extend(action_texts(&setup_actions(setup.local)));
    if setup.ingress == gumgum_core::IngressMode::Cloudflare {
        actions.push(format!(
            "authorize Cloudflare for {} and configure Cloudflare ingress",
            setup.root_domain
        ));
    }
    actions.push(format!("save server {} ({})", setup.name, setup.host));
    actions.push("initialize built-in db/kv/queue/bucket/secret providers".to_owned());
    actions
}

fn remove_server(host_or_name: &str) -> gumgum_core::Result<ServerMutationReport> {
    let removed = ConfigStore::from_home_env()?.remove_server(host_or_name)?;
    match removed {
        Some(server) => {
            let action = format!("removed server {} ({})", host_or_name, server.host);
            Ok(ServerMutationReport {
                ok: true,
                dry_run: false,
                action: "rm".to_owned(),
                message: format!("server {} removed", server.name),
                server: Some(server),
                actions: vec![action],
            })
        }
        None => Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!("unknown GumGum.dev server {host_or_name}"),
        )
        .next_command("gumgum server list")
        .build()),
    }
}

fn ping_target(args: &crate::PingArgs) -> gumgum_core::Result<String> {
    args.host
        .clone()
        .or_else(|| args.target.clone())
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::InvalidArgs,
                "server host/name is required for ping",
            )
            .next_command("gumgum server ping --host <host-or-name>")
            .build()
        })
}

fn server_command_target(
    legacy_name: Option<String>,
    host: Option<String>,
    command: &str,
) -> gumgum_core::Result<String> {
    host.or(legacy_name).ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            format!("server host/name is required for {command}"),
        )
        .next_command(format!("gumgum server {command} --host <host-or-name>"))
        .build()
    })
}

fn server_capabilities_target(
    legacy_name: Option<String>,
    args: &crate::ServerCapabilitiesArgs,
) -> gumgum_core::Result<String> {
    if let Some(action) = &args.action {
        if action != "list" {
            return Err(GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::InvalidArgs,
                format!("unknown server capabilities action {action}"),
            )
            .next_command("gumgum server capabilities list --host <host-or-name>")
            .build());
        }
    }
    args.host.clone().or(legacy_name).ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            "server host/name is required for capabilities",
        )
        .next_command("gumgum server capabilities list --host <host-or-name>")
        .next_command("gumgum server <name> capabilities")
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
        ingress: gumgum_core::IngressMode::Direct,
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
    progress(quiet, "checking required smoke capabilities");
    let version = ServerClient::new(setup.host.clone()).version().await?;
    require_smoke_readiness(&server.name, &version)?;
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

fn require_smoke_readiness(name: &str, report: &DaemonVersionReport) -> gumgum_core::Result<()> {
    let required = smoke_required_capabilities();
    let missing = missing_capabilities(&report.capabilities, &required);
    if missing.is_empty() {
        return Ok(());
    }
    Err(GumgumError::structured(
        Subsystem::Cli,
        ErrorCode::NotImplemented,
        format!(
            "gumgumd on {name} is missing required capabilities: {}",
            missing.join(", ")
        ),
    )
    .next_command(format!("gumgum server capabilities list --host {name}"))
    .next_command(format!("gumgum --dry-run server {name} upgrade"))
    .next_command(format!(
        "gumgum server capabilities list --host {name} --require {}",
        required.join(",")
    ))
    .build())
}

fn require_capabilities(
    name: &str,
    report: &DaemonVersionReport,
    required: &[String],
) -> gumgum_core::Result<()> {
    let missing = missing_capabilities(&report.capabilities, required);
    if missing.is_empty() {
        return Ok(());
    }
    Err(GumgumError::structured(
        Subsystem::Cli,
        ErrorCode::NotImplemented,
        format!(
            "gumgumd on {name} is missing required capabilities: {}",
            missing.join(", ")
        ),
    )
    .next_command(format!("gumgum server capabilities list --host {name}"))
    .next_command(format!("gumgum --dry-run server {name} upgrade"))
    .build())
}

fn capability_lines(report: &DaemonVersionReport, required: &[String]) -> Vec<String> {
    let mut lines = vec![format!("gumgumd {} ({})", report.version, report.git_sha)];
    lines.push(format!("target: {}", report.target));
    lines.push("capabilities:".to_owned());
    lines.extend(report.capabilities.iter().map(|capability| {
        let marker = if required.iter().any(|required| required == capability) {
            "*"
        } else {
            "-"
        };
        format!("  {marker} {capability}")
    }));
    if !required.is_empty() {
        let missing = missing_capabilities(&report.capabilities, required);
        if missing.is_empty() {
            lines.push("required capabilities: ok".to_owned());
        } else {
            lines.push(format!(
                "required capabilities: missing {}",
                missing.join(", ")
            ));
            lines.push("next: gumgum server capabilities list --host <name>".to_owned());
            lines.push("next: gumgum --dry-run server <name> upgrade".to_owned());
        }
    }
    lines
}

fn missing_capabilities(capabilities: &[String], required: &[String]) -> Vec<String> {
    required
        .iter()
        .filter(|required| {
            !capabilities
                .iter()
                .any(|capability| capability == *required)
        })
        .cloned()
        .collect()
}

fn smoke_required_capabilities() -> Vec<String> {
    vec![
        "gumgum:events".to_owned(),
        "gumgum:rollback:revision_id".to_owned(),
        "gumgum:objects:create_preview".to_owned(),
        "gumgum:bindings:create_preview".to_owned(),
        "gumgum:bindings:delete".to_owned(),
        "gumgum:objects:delete".to_owned(),
        "gumgum:deployments:delete".to_owned(),
    ]
}

fn required_capabilities_from_args(required: &[String]) -> Vec<String> {
    required
        .iter()
        .map(|capability| capability.trim())
        .filter(|capability| !capability.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn server_upgrade_actions(dry_run: bool) -> Vec<String> {
    let mut actions = vec![
        "ssh into server".to_owned(),
        "run published GumGum.dev installer".to_owned(),
        "restart gumgumd via remote setup".to_owned(),
        "check gumgumd health".to_owned(),
        "verify required smoke capabilities".to_owned(),
    ];
    if dry_run {
        actions.insert(0, "preview only; no ssh command will run".to_owned());
    }
    actions
}

fn print_server_list(servers: &[ServerRecord]) {
    if servers.is_empty() {
        println!("No GumGum.dev servers configured.");
        println!("Run: gumgum server add <host> --name <name> --domain <domain>");
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

fn print_server_mutation_report(report: &ServerMutationReport) {
    println!("{}", report.message);
    if let Some(server) = &report.server {
        println!("name: {}", server.name);
        println!("host: {}", server.host);
        if !server.root_domain.is_empty() {
            println!("root domain: {}", server.root_domain);
        }
        if !server.test_domain.is_empty() {
            println!("test domain: {}", server.test_domain);
        }
        println!("health: {}", server.health_url);
    }
    if !report.actions.is_empty() {
        println!("Actions:");
        for action in &report.actions {
            println!("  - {action}");
        }
    }
}

fn print_ping_report(report: &PingReport) {
    println!(
        "gumgumd: {} ({})",
        if report.ok { "healthy" } else { "unhealthy" },
        report.health_url
    );
    println!("host: {}", report.host);
    if let Some(active) = report.service_active {
        println!("service: active={active}");
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
        let object_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.kind == "object" || node.kind == "global_object")
            .collect::<Vec<_>>();
        let binding_workers: BTreeMap<_, _> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == "binds")
            .map(|edge| (edge.to.as_str(), edge.from.as_str()))
            .collect();
        let mut object_bindings: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for edge in graph.edges.iter().filter(|edge| edge.kind == "projects_as") {
            let worker = binding_workers
                .get(edge.from.as_str())
                .and_then(|worker_id| worker_id.strip_prefix("worker/"))
                .unwrap_or("unknown");
            let binding = edge
                .from
                .strip_prefix(&format!("binding/{worker}/"))
                .unwrap_or(edge.from.as_str());
            object_bindings
                .entry(edge.to.as_str())
                .or_default()
                .insert(format!("{worker}.{binding}"));
        }
        let bound_objects = object_nodes
            .iter()
            .filter(|node| object_bindings.contains_key(node.id.as_str()))
            .count();
        let unbound_objects = object_nodes.len().saturating_sub(bound_objects);
        lines.push(format!(
            "Desired graph: workers={workers} routes={routes} objects={} bound_objects={bound_objects} unbound_objects={unbound_objects}",
            object_nodes.len()
        ));
        for route in graph.nodes.iter().filter(|node| node.kind == "route") {
            lines.push(format!("  - route {}", route.label));
        }
        for object in object_nodes {
            if let Some(bindings) = object_bindings.get(object.id.as_str()) {
                let bindings = bindings.iter().cloned().collect::<Vec<_>>().join(", ");
                lines.push(format!("  - object {} (bound: {bindings})", object.label));
            } else {
                lines.push(format!("  - object {} (unbound)", object.label));
            }
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
    fn capability_lines_explain_explicit_requirements() {
        let report = DaemonVersionReport {
            ok: true,
            version: "0.1.0".to_owned(),
            git_sha: "abc123".to_owned(),
            target: "x86_64".to_owned(),
            capabilities: vec!["gumgum:events".to_owned()],
        };

        let lines = capability_lines(
            &report,
            &[
                "gumgum:events".to_owned(),
                "gumgum:bindings:delete".to_owned(),
                "gumgum:objects:delete".to_owned(),
            ],
        );

        assert!(lines.contains(&"  * gumgum:events".to_owned()));
        assert!(
            lines.contains(
                &"required capabilities: missing gumgum:bindings:delete, gumgum:objects:delete"
                    .to_owned()
            )
        );
        assert!(lines.contains(&"next: gumgum --dry-run server <name> upgrade".to_owned()));
    }

    #[test]
    fn require_capabilities_errors_with_upgrade_hint() {
        let report = DaemonVersionReport {
            ok: true,
            version: "0.1.0".to_owned(),
            git_sha: "abc123".to_owned(),
            target: "x86_64".to_owned(),
            capabilities: vec!["gumgum:events".to_owned()],
        };

        let report = require_capabilities(
            "starbase2",
            &report,
            &[
                "gumgum:events".to_owned(),
                "gumgum:bindings:delete".to_owned(),
            ],
        )
        .unwrap_err()
        .to_report();

        assert!(report.message.contains("missing required capabilities"));
        assert!(
            report
                .next_commands
                .contains(&"gumgum --dry-run server starbase2 upgrade".to_owned())
        );
    }

    #[test]
    fn server_upgrade_actions_explain_dry_run_safety() {
        assert_eq!(
            server_upgrade_actions(true)[0],
            "preview only; no ssh command will run"
        );
        assert!(
            server_upgrade_actions(true).contains(&"verify required smoke capabilities".to_owned())
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
            edges: vec![
                GraphEdge {
                    from: "worker/api".to_owned(),
                    to: "binding/api/DATABASE_URL".to_owned(),
                    kind: "binds".to_owned(),
                },
                GraphEdge {
                    from: "binding/api/DATABASE_URL".to_owned(),
                    to: "object/db/visits".to_owned(),
                    kind: "projects_as".to_owned(),
                },
            ],
        };

        let lines = status_summary_lines(&ping, Some(&providers), Some(&graph));

        assert!(lines.contains(&"gumgumd: healthy (http://starbase2:7777/healthz)".to_owned()));
        assert!(lines.contains(&"Providers: 0/1 running".to_owned()));
        assert!(lines.iter().any(|line| line.contains("provider warning")));
        assert!(
            lines.contains(
                &"Desired graph: workers=1 routes=1 objects=1 bound_objects=1 unbound_objects=0"
                    .to_owned()
            )
        );
        assert!(lines.contains(&"  - route api.visit-counter.leostera.test".to_owned()));
        assert!(lines.contains(&"  - object visits (bound: api.DATABASE_URL)".to_owned()));
    }
}
