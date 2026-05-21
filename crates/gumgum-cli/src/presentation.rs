use crate::deploy_command::{DeployOutput, DeployReport, WorkspaceDeployReport};
use gumgum_api::ObjectReport;

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
            println!("  - {action}");
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
    lines.push(format!("Image: {}", report.image));
    lines.push(format!("Container: {}", report.container));
    lines.push("Plan:".to_owned());
    for level in &report.plan_graph.execution_levels {
        lines.push(format!("  - {}", level.join(", ")));
    }
    if !report.dry_run {
        lines.push(report.message.clone());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::PlanGraph;

    #[test]
    fn deploy_lines_include_health_status_url() {
        let report = DeployReport {
            ok: true,
            dry_run: false,
            path: "api/gumgum.toml".to_owned(),
            worker: "api".to_owned(),
            host: Some("starbase2".to_owned()),
            build_context: Some("api".to_owned()),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            port: 3000,
            routes: vec!["api.visit-counter.leostera.test".to_owned()],
            health_url: Some("http://api.visit-counter.leostera.test/healthz".to_owned()),
            plan: Vec::new(),
            plan_graph: PlanGraph::default(),
            message: "deployed api; health verified".to_owned(),
        };

        let lines = deploy_report_lines(&report);

        assert!(
            lines.contains(&"Health: http://api.visit-counter.leostera.test/healthz".to_owned())
        );
        assert!(lines.contains(&"deployed api; health verified".to_owned()));
    }
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
