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
        println!("Worker: {}", report.worker);
        if let Some(host) = &report.host {
            println!("Host: {host}");
        }
        for route in &report.routes {
            println!("Route: {route}");
        }
        println!("Image: {}", report.image);
        println!("Container: {}", report.container);
        println!("Plan:");
        for level in &report.plan_graph.execution_levels {
            println!("  - {}", level.join(", "));
        }
        if !report.dry_run {
            println!("{}", report.message);
        }
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
