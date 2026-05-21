use crate::{PublishArgs, print_value, resolve_server};
use gumgum_core::{
    DeploymentDescriptor, ErrorCode, GumgumError, PlanEdge, PlanGraph, PlanNode, Subsystem,
    load_worker_path, validate_path,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct PublishReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) worker: String,
    pub(crate) local_routes_preserved: Vec<String>,
    pub(crate) public_routes: Vec<String>,
    pub(crate) tunnel: String,
    pub(crate) plan_graph: PlanGraph,
    pub(crate) plan: Vec<String>,
    pub(crate) message: String,
}

pub(crate) async fn publish(
    args: PublishArgs,
    dry_run: bool,
    json: bool,
) -> gumgum_core::Result<()> {
    if !dry_run {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            "publish currently supports dry-run planning only",
        )
        .next_command("gumgum --dry-run publish <worker>")
        .build());
    }
    let server = resolve_server(args.host)?;
    let path = publish_manifest_path(&args.target)?;
    let manifest = load_worker_path(&path)?;
    validate_path(&path)?;
    let local = DeploymentDescriptor::from_manifest(&path, &manifest, Some(&server), false);
    let public = DeploymentDescriptor::from_manifest(&path, &manifest, Some(&server), true);
    let local_routes = local.routes;
    let public_routes = args
        .public_domain
        .map(|domain| vec![domain])
        .unwrap_or(public.routes);
    let report = PublishReport {
        ok: true,
        dry_run: true,
        worker: manifest.worker.name.clone(),
        local_routes_preserved: local_routes.clone(),
        public_routes: public_routes.clone(),
        tunnel: args.tunnel,
        plan_graph: publish_plan_graph(&manifest.worker.name, &local_routes, &public_routes),
        plan: publish_plan_lines(&manifest.worker.name, &local_routes, &public_routes),
        message: "publish dry-run; no public route changed".to_owned(),
    };
    if json {
        print_value(true, &report);
    } else {
        for line in publish_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn publish_manifest_path(target: &Path) -> gumgum_core::Result<PathBuf> {
    if target.is_dir() {
        return Ok(target.join("gumgum.toml"));
    }
    if target.exists() {
        return Ok(target.to_path_buf());
    }
    let worker_manifest = target.join("gumgum.toml");
    if worker_manifest.exists() {
        return Ok(worker_manifest);
    }
    Ok(target.to_path_buf())
}

fn publish_plan_lines(
    worker: &str,
    local_routes: &[String],
    public_routes: &[String],
) -> Vec<String> {
    let mut lines = vec![
        format!("load worker {worker}"),
        "read current local route state".to_owned(),
    ];
    for route in local_routes {
        lines.push(format!("preserve local route {route}"));
    }
    for route in public_routes {
        lines.push(format!("would publish public route {route}"));
    }
    lines.push("would configure BYO tunnel/provider; no local deploy is changed".to_owned());
    lines
}

fn publish_plan_graph(
    worker: &str,
    local_routes: &[String],
    public_routes: &[String],
) -> PlanGraph {
    let worker_id = format!("worker/{worker}");
    let mut nodes = vec![
        PlanNode::new(&worker_id, "worker", worker, "read deployed worker"),
        PlanNode::new(
            "route/local",
            "local_route",
            local_routes.join(", "),
            "preserve local route",
        ),
        PlanNode::new(
            "publish/public",
            "public_route",
            public_routes.join(", "),
            "plan public route",
        ),
        PlanNode::new("tunnel/byo", "tunnel", "BYO tunnel", "plan tunnel mapping"),
    ];
    if local_routes.is_empty() {
        nodes[1] = PlanNode::new("route/local", "local_route", "none", "preserve local route");
    }
    let edges = vec![
        PlanEdge::new(&worker_id, "route/local", "keeps"),
        PlanEdge::new(&worker_id, "publish/public", "publishes_as"),
        PlanEdge::new("publish/public", "tunnel/byo", "requires"),
    ];
    PlanGraph::new(nodes, edges)
}

fn publish_lines(report: &PublishReport) -> Vec<String> {
    let mut lines = vec![format!("Worker: {}", report.worker)];
    for route in &report.local_routes_preserved {
        lines.push(format!("Local route preserved: {route}"));
    }
    for route in &report.public_routes {
        lines.push(format!("Public route planned: {route}"));
    }
    lines.push(format!("Tunnel: {}", report.tunnel));
    lines.push("Plan:".to_owned());
    lines.extend(report.plan.iter().map(|step| format!("  - {step}")));
    lines.push(
        "Safety: local deploy never publishes publicly without this explicit publish flow"
            .to_owned(),
    );
    lines.push(report.message.clone());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_lines_preserve_local_and_plan_public_route() {
        let report = PublishReport {
            ok: true,
            dry_run: true,
            worker: "api".to_owned(),
            local_routes_preserved: vec!["api.visit-counter.leostera.test".to_owned()],
            public_routes: vec!["api.visit-counter.leostera.dev".to_owned()],
            tunnel: "byo".to_owned(),
            plan_graph: PlanGraph::default(),
            plan: vec!["would publish public route api.visit-counter.leostera.dev".to_owned()],
            message: "publish dry-run; no public route changed".to_owned(),
        };

        let lines = publish_lines(&report);

        assert!(
            lines.contains(&"Local route preserved: api.visit-counter.leostera.test".to_owned())
        );
        assert!(lines.contains(&"Public route planned: api.visit-counter.leostera.dev".to_owned()));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("local deploy never publishes publicly"))
        );
    }

    #[test]
    fn publish_graph_has_explicit_public_route_state() {
        let graph = publish_plan_graph(
            "api",
            &["api.visit-counter.leostera.test".to_owned()],
            &["api.visit-counter.leostera.dev".to_owned()],
        );

        assert!(graph.nodes.iter().any(|node| node.kind == "public_route"));
        assert!(graph.nodes.iter().any(|node| node.kind == "local_route"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "publishes_as"));
    }
}
