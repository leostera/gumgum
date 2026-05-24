#![allow(clippy::items_after_test_module)]

use crate::deploy_command::{DeployOutput, DeployReport, WorkspaceDeployReport};
use gumgum_api::ObjectReport;
use gumgum_core::{ActionScope, CoreAction, SetupStep};

pub(crate) struct Presenter;

impl Presenter {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn deploy_output(&self, output: &DeployOutput) {
        match output {
            DeployOutput::Worker(report) => self.deploy_report(report),
            DeployOutput::Workspace(report) => self.workspace_deploy_report(report),
            DeployOutput::Delete(report) => self.deploy_delete_report(report),
        }
    }

    pub(crate) fn object_report(&self, report: &ObjectReport) {
        println!("{} '{}' ready", report.kind, report.name);
        println!("DNS: {}", report.dns);
        println!("Provider: {}", report.provider);

        let examples = if report.connection_examples.is_empty() {
            connection_examples(&report.kind, &report.name, &report.dns)
        } else {
            report.connection_examples.clone()
        };

        if !examples.is_empty() {
            println!("\nConnect:");
            for example in examples {
                println!("  {example}");
            }
        }
    }

    fn deploy_delete_report(&self, report: &gumgum_api::DeployApplyReport) {
        println!("Worker: {}", report.worker);
        println!("{}", report.message);
        for action in &report.actions {
            println!("  - {}", action_text(action));
        }
    }

    fn workspace_deploy_report(&self, report: &WorkspaceDeployReport) {
        println!("Workspace: {}", report.workspace);
        println!("Plan:");
        for step in &report.plan {
            println!("  - {step}");
        }
        if !report.dry_run {
            println!("{}", report.message);
        }
    }

    fn deploy_report(&self, report: &DeployReport) {
        for line in deploy_report_lines(report) {
            println!("{line}");
        }
    }
}

fn deploy_report_lines(report: &DeployReport) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Worker: {}", report.worker));
    if let Some(host) = &report.host {
        lines.push(format!("Host: {host}"));
    }
    for route in &report.routes {
        lines.push(format!("Route: {route}"));
    }
    if let Some(health_url) = &report.health_url {
        lines.push(format!("Health: {health_url}"));
    }
    for artifact in &report.grafana {
        lines.push(format!(
            "Grafana {}: {} ({})",
            artifact.kind, artifact.name, artifact.path
        ));
    }
    lines.push(format!("Image: {}", report.image));
    lines.push(format!("Container: {}", report.container));
    lines.push("Plan:".to_owned());
    for level in &report.plan_graph.execution_levels {
        lines.push(format!("  - {}", level.join(", ")));
    }
    lines.extend(deploy_impact_lines(report));
    if !report.dry_run {
        lines.push(report.message.clone());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{PlanGraph, PlanNode};

    #[test]
    fn deploy_lines_include_health_status_url() {
        let report = DeployReport {
            ok: true,
            dry_run: false,
            path: "api/gumgum.toml".to_owned(),
            worker: "api".to_owned(),
            project: None,
            domain: None,
            host: Some("starbase2".to_owned()),
            build_context: Some("api".to_owned()),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            port: 3000,
            routes: vec!["api.visit-counter.leostera.test".to_owned()],
            health_url: Some("http://api.visit-counter.leostera.test/healthz".to_owned()),
            grafana: Vec::new(),
            plan: Vec::new(),
            plan_graph: PlanGraph::default(),
            events: Vec::new(),
            message: "deployed api; health verified".to_owned(),
        };

        let lines = deploy_report_lines(&report);

        assert!(
            lines.contains(&"Health: http://api.visit-counter.leostera.test/healthz".to_owned())
        );
        assert!(lines.contains(&"deployed api; health verified".to_owned()));
    }

    #[test]
    fn deploy_lines_include_grafana_artifacts() {
        let report = DeployReport {
            ok: true,
            dry_run: true,
            path: "api/gumgum.toml".to_owned(),
            worker: "api".to_owned(),
            project: None,
            domain: None,
            host: Some("starbase2".to_owned()),
            build_context: Some("api".to_owned()),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            port: 3000,
            routes: Vec::new(),
            health_url: None,
            grafana: vec![crate::deploy_command::GrafanaArtifactPlan {
                kind: "dashboard".to_owned(),
                name: "kava-fund / API Overview".to_owned(),
                folder_path: vec!["kava.fund".to_owned(), "kava-fund".to_owned()],
                path: "grafana/dashboards/api-overview.json".to_owned(),
            }],
            plan: Vec::new(),
            plan_graph: PlanGraph::default(),
            events: Vec::new(),
            message: String::new(),
        };

        let lines = deploy_report_lines(&report);

        assert!(lines.contains(&
            "Grafana dashboard: kava-fund / API Overview (grafana/dashboards/api-overview.json)"
                .to_owned()
        ));
    }

    #[test]
    fn deploy_lines_explain_impact_and_rollback() {
        let mut plan_graph = PlanGraph::default();
        plan_graph.nodes.push(PlanNode::new(
            "worker/api",
            "worker",
            "api",
            "build and push worker image",
        ));
        let report = DeployReport {
            ok: true,
            dry_run: true,
            path: "api/gumgum.toml".to_owned(),
            worker: "api".to_owned(),
            project: None,
            domain: None,
            host: Some("starbase2".to_owned()),
            build_context: Some("api".to_owned()),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            port: 3000,
            routes: vec!["api.visit-counter.leostera.test".to_owned()],
            health_url: Some("http://api.visit-counter.leostera.test/healthz".to_owned()),
            grafana: Vec::new(),
            plan: Vec::new(),
            plan_graph,
            events: Vec::new(),
            message: String::new(),
        };

        let lines = deploy_report_lines(&report);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("will touch: worker:api"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("will not touch: unrelated containers"))
        );
        assert!(lines.contains(&"  rollback: gumgum rollback --worker api --preview".to_owned()));
    }
}

fn deploy_impact_lines(report: &DeployReport) -> Vec<String> {
    let mut lines = vec!["Impact:".to_owned()];
    if report.plan_graph.nodes.is_empty() {
        lines.push("  will touch: worker manifest only".to_owned());
    } else {
        let touched = report
            .plan_graph
            .nodes
            .iter()
            .map(|node| format!("{}:{} ({})", node.kind, node.label, node.action))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("  will touch: {touched}"));
    }
    lines.push(
        "  will not touch: unrelated containers, providers, or objects outside this plan"
            .to_owned(),
    );
    lines.push(
        "  risk: image build/push, container replacement, route and health verification".to_owned(),
    );
    lines.push(format!(
        "  rollback: gumgum rollback --worker {} --preview",
        report.worker
    ));
    lines
}

fn connection_examples(kind: &str, name: &str, dns: &str) -> Vec<String> {
    match kind {
        "db" | "database" => vec![
            format!("psql postgres://{name}:<password>@{dns}:5432/{name}"),
            format!("pgAdmin host={dns} port=5432 database={name} username={name}"),
        ],
        "kv" => vec![
            format!("redis-cli -u redis://{dns}:6379/0"),
            format!("RedisInsight host={dns} port=6379 database=0"),
        ],
        _ => Vec::new(),
    }
}

pub(crate) fn action_text(action: &CoreAction) -> String {
    match action {
        CoreAction::CliMessage { message } => message.clone(),
        CoreAction::SetupStep { step } => match step {
            SetupStep::CreateLocalDirectories => {
                "create ~/.gumgum/bin and ~/.gumgum/daemon".to_owned()
            }
            SetupStep::InstallRunningBinary => {
                "install running gumgum binary into ~/.gumgum/bin".to_owned()
            }
            SetupStep::WriteUserSystemdService => "write gumgumd user-systemd service".to_owned(),
            SetupStep::EnableRestartDaemon => "enable and restart gumgumd".to_owned(),
            SetupStep::CheckLocalHealth => "check http://127.0.0.1:7777/healthz".to_owned(),
            SetupStep::SshIntoHost => "ssh into host".to_owned(),
            SetupStep::RunRemoteInstaller => {
                "run curl -fsSL https://get.gumgum.dev | sh".to_owned()
            }
            SetupStep::RunRemoteSetup => "run ~/.gumgum/bin/gumgum setup on the host".to_owned(),
            SetupStep::ExitSsh => "exit ssh".to_owned(),
            SetupStep::SaveServerLocally => "save server locally".to_owned(),
            SetupStep::CheckRemoteHealth => "check http://<host>:7777/healthz".to_owned(),
        },
        CoreAction::PreviewOnly { scope } => {
            format!("preview only; no {} changed", scope_noun(*scope))
        }
        CoreAction::AlreadyBound { worker, binding } => {
            format!("still bound to worker {worker} as {binding}")
        }
        CoreAction::ProviderCredentialsRequired { provider } => {
            format!("provider credentials are required for {provider}")
        }
        CoreAction::ReconcileFailed { scope, error } => {
            format!("{} reconcile failed: {error}", scope_noun(*scope))
        }
        CoreAction::Planned { target, action } => format!("planned {action} for {target}"),
        CoreAction::ProviderConfigured {
            capability,
            provider,
        } => format!("configured {capability} provider {provider}"),
        CoreAction::ProviderObjectDesiredRemoved { capability, name } => format!(
            "removed desired {capability} object {name}; provider cleanup is not implemented yet"
        ),
        CoreAction::ProviderContainerCreated {
            provider,
            container,
        } => format!("created {provider} provider container {container}"),
        CoreAction::ProviderContainerStarted { provider } => {
            format!("started existing {provider} provider")
        }
        CoreAction::ProviderContainerRecreated { provider } => {
            format!("recreated {provider} provider with configured password")
        }
        CoreAction::PlatformServiceCreated {
            provider,
            container,
        } => format!("created platform service {container} ({provider})"),
        CoreAction::PlatformServiceStarted { container } => format!("started existing {container}"),
        CoreAction::PlatformSecretServiceCreated {
            provider,
            container,
        } => format!("created platform secret service {container} ({provider})"),
        CoreAction::DnsPublished { dns, provider } => format!("published DNS {dns} to {provider}"),
        CoreAction::DnsRemoved { dns, provider } => format!("removed DNS {dns} from {provider}"),
        CoreAction::DatabaseRoleEnsured { role } => format!("ensured database role {role}"),
        CoreAction::DatabaseAlreadyExists { database } => {
            format!("database {database} already exists")
        }
        CoreAction::DatabaseCreated { database } => format!("created database {database}"),
        CoreAction::DatabaseGranted { database, role } => {
            format!("granted database {database} to role {role}")
        }
        CoreAction::DatabaseDropped { database } => format!("dropped database {database}"),
        CoreAction::DatabaseAlreadyAbsent { database } => {
            format!("database {database} was already absent")
        }
        CoreAction::RedisPrefixReserved { prefix } => {
            format!("reserved Redis key prefix {prefix}:")
        }
        CoreAction::RedisPrefixReleased { prefix } => {
            format!("released Redis key prefix {prefix}:")
        }
        CoreAction::BucketEnsured { bucket, provider } => {
            format!("ensured bucket {bucket} on {provider}")
        }
        CoreAction::BucketDeleted { bucket, provider } => {
            format!("deleted bucket {bucket} from {provider}")
        }
        CoreAction::BucketObjectUploaded {
            bucket,
            path,
            provider,
        } => format!("uploaded {bucket}/{path} to {provider}"),
        CoreAction::BucketObjectRemoved {
            bucket,
            path,
            provider,
        } => format!("removed {bucket}/{path} from {provider}"),
        CoreAction::BucketObjectCopied {
            source,
            destination,
            provider,
        } => format!("copied {source} to {destination} in {provider}"),
        CoreAction::BucketObjectsSynced {
            source,
            destination,
            provider,
        } => format!("synced {source} to {destination} in {provider}"),
        CoreAction::QueueTopicEnsured { topic, provider } => {
            format!("ensured topic {topic} on {provider}")
        }
        CoreAction::QueueTopicDeleted { topic, provider } => {
            format!("deleted topic {topic} from {provider}")
        }
        CoreAction::PrometheusScrapeConfigured {
            worker,
            environment,
            container,
            port,
            metrics_path,
        } => format!(
            "configured Prometheus scrape for {worker}@{environment} at {container}:{port}{metrics_path}"
        ),
        CoreAction::GrafanaDatasourceCreated { name } => {
            format!("created Grafana datasource {name}")
        }
        CoreAction::GrafanaDatasourceUpdated { name } => {
            format!("updated Grafana datasource {name}")
        }
        CoreAction::GrafanaDashboardApplied { name } => format!("applied Grafana dashboard {name}"),
        CoreAction::DeploymentContainerMatches { container } => {
            format!("container {container} already matches desired image, route, and bindings")
        }
        CoreAction::ImagePulled { image } => format!("pull {image}"),
        CoreAction::NetworkCreated { network } => format!("create environment network {network}"),
        CoreAction::DeploymentEnvironmentProjected { vars } => format!("project {vars} env var(s)"),
        CoreAction::DeploymentContainerStarted { container } => {
            format!("start new deployment container {container}")
        }
        CoreAction::ContainerConnectedToNetwork { container, network } => {
            format!("connect {container} to {network}")
        }
        CoreAction::DeploymentContainerHealthy { container } => {
            format!("new deployment container {container} is healthy; removing old containers")
        }
        CoreAction::DeploymentContainerRemoved { container } => {
            format!("removed deployment container {container}")
        }
        CoreAction::CloudflareDnsCnameDeleted { hostname } => {
            format!("delete Cloudflare DNS CNAME {hostname}")
        }
        CoreAction::CloudflareDnsCnameAbsent { hostname } => {
            format!("Cloudflare DNS CNAME {hostname} was already absent")
        }
        CoreAction::CloudflareDnsCnameUnmanaged { hostname } => format!(
            "Cloudflare DNS CNAME {hostname} was not deleted because it is not marked managed-by=gumgum"
        ),
        CoreAction::CloudflareConnectorEnsured { container } => {
            format!("ensure Cloudflare connector container {container}")
        }
        CoreAction::CloudflareConnectorStarted { container } => {
            format!("started Cloudflare connector container {container}")
        }
        CoreAction::CloudflareTunnelEnsured { tunnel } => {
            format!("ensure Cloudflare tunnel {tunnel}")
        }
        CoreAction::CloudflareTunnelRouteEnsured { hostname, service } => {
            format!("ensure Cloudflare tunnel route {hostname} -> {service}")
        }
        CoreAction::CloudflareDnsCnameEnsured { hostname, target } => {
            format!("ensure Cloudflare DNS CNAME {hostname} -> {target}")
        }
        CoreAction::ManualDnsRequired { hostname, domain } => {
            format!("manual DNS required for {hostname} under {domain}")
        }
        CoreAction::ManualDnsCleanupRequired { hostname, domain } => {
            format!("manual DNS cleanup required for stale route {hostname} under {domain}")
        }
        CoreAction::CloudflareDirectDnsUnsupported { hostname } => {
            format!("Cloudflare direct DNS for {hostname} is not implemented yet")
        }
        CoreAction::NoManagedDomainForStaleRoute { hostname } => {
            format!("no managed domain matches stale route {hostname}; DNS was not changed")
        }
    }
}

pub(crate) fn action_texts(actions: &[CoreAction]) -> Vec<String> {
    actions.iter().map(action_text).collect()
}

fn scope_noun(scope: ActionScope) -> &'static str {
    match scope {
        ActionScope::Objects => "objects",
        ActionScope::Bindings => "bindings",
        ActionScope::Deployment => "deployments",
        ActionScope::Provider => "provider",
        ActionScope::Reconcile => "reconcile",
    }
}
