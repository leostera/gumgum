#![allow(clippy::items_after_test_module)]

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
