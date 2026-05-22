use crate::{PublishArgs, print_value, resolve_server};
use gumgum_api::ServerRecord;
use gumgum_core::{
    DeploymentDescriptor, ErrorCode, GumgumError, ManifestKind, PlanEdge, PlanGraph, PlanNode,
    Subsystem, WorkerManifest, load_worker_path, load_workspace_path, validate_path,
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

#[derive(Debug, Serialize)]
pub(crate) struct WorkspacePublishReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) workspace: String,
    pub(crate) workers: Vec<PublishReport>,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum PublishOutput {
    Worker(PublishReport),
    Workspace(WorkspacePublishReport),
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
    let server = resolve_server(args.host.clone())?;
    let output = publish_output(args, &server)?;
    if json {
        print_value(true, &output);
    } else {
        for line in publish_output_lines(&output) {
            println!("{line}");
        }
    }
    Ok(())
}

fn publish_output(args: PublishArgs, server: &ServerRecord) -> gumgum_core::Result<PublishOutput> {
    let path = publish_manifest_path(&args.target)?;
    match validate_path(&path)?.manifest_kind {
        ManifestKind::Worker => {
            let manifest = load_worker_path(&path)?;
            Ok(PublishOutput::Worker(publish_report(
                &path,
                &manifest,
                server,
                &args.tunnel,
                args.public_domain,
            )))
        }
        ManifestKind::Workspace => {
            if args.public_domain.is_some() {
                return Err(GumgumError::structured(
                    Subsystem::Cli,
                    ErrorCode::InvalidArgs,
                    "--public-domain requires a single worker target",
                )
                .next_command("gumgum --dry-run publish api/gumgum.toml --public-domain <domain>")
                .build());
            }
            let workspace = load_workspace_path(&path)?;
            let root = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let mut workers = Vec::new();
            for member in workspace.members() {
                let member_path = root.join(member).join("gumgum.toml");
                let manifest = load_worker_path(&member_path)?;
                workers.push(publish_report(
                    &member_path,
                    &manifest,
                    server,
                    &args.tunnel,
                    None,
                ));
            }
            Ok(PublishOutput::Workspace(WorkspacePublishReport {
                ok: true,
                dry_run: true,
                workspace: workspace.namespace_name().to_owned(),
                workers,
                message: "workspace publish dry-run; no public routes changed".to_owned(),
            }))
        }
    }
}

fn publish_report(
    path: &Path,
    manifest: &WorkerManifest,
    server: &ServerRecord,
    tunnel: &str,
    public_domain: Option<String>,
) -> PublishReport {
    let local = DeploymentDescriptor::from_manifest(path, manifest, Some(server), false);
    let public = DeploymentDescriptor::from_manifest(path, manifest, Some(server), true);
    let manifest_local_routes = manifest
        .ingress
        .iter()
        .filter_map(|ingress| ingress.local_domain.clone())
        .filter(|route| !route.is_empty())
        .collect::<Vec<_>>();
    let manifest_public_routes = public.routes.clone();
    let local_routes = if manifest_local_routes.is_empty() {
        local.routes
    } else {
        manifest_local_routes
    };
    let public_routes = public_domain.map(|domain| vec![domain]).unwrap_or_else(|| {
        if manifest_public_routes.is_empty() {
            public.routes
        } else {
            manifest_public_routes
        }
    });
    PublishReport {
        ok: true,
        dry_run: true,
        worker: manifest.worker.name.clone(),
        local_routes_preserved: local_routes.clone(),
        public_routes: public_routes.clone(),
        tunnel: tunnel.to_owned(),
        plan_graph: publish_plan_graph(&manifest.worker.name, &local_routes, &public_routes),
        plan: publish_plan_lines(&manifest.worker.name, &local_routes, &public_routes),
        message: "publish dry-run; no public route changed".to_owned(),
    }
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

fn publish_output_lines(output: &PublishOutput) -> Vec<String> {
    match output {
        PublishOutput::Worker(report) => publish_lines(report),
        PublishOutput::Workspace(report) => {
            let mut lines = vec![format!("Workspace: {}", report.workspace)];
            for worker in &report.workers {
                lines.push(format!("Worker: {}", worker.worker));
                lines.extend(
                    publish_lines(worker)
                        .into_iter()
                        .skip(1)
                        .map(|line| format!("  {line}")),
                );
            }
            lines.push(report.message.clone());
            lines
        }
    }
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
    use std::fs;

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

    #[test]
    fn publish_output_expands_workspace_members() {
        let dir = temp_dir("workspace");
        fs::create_dir_all(dir.join("api")).unwrap();
        fs::create_dir_all(dir.join("worker")).unwrap();
        fs::write(
            dir.join("gumgum.toml"),
            "[project]\nname = \"visit-counter\"\ndomain = \"visitcounter.dev\"\n\n[workspace]\nmembers = [\"api\", \"worker\"]\n",
        )
        .unwrap();
        for worker in ["api", "worker"] {
            fs::write(
                dir.join(worker).join("gumgum.toml"),
                format!(
                    "[project]\nnamespace = \"visit-counter\"\n\n[worker]\nname = \"{worker}\"\nbuild_context = \".\"\nport = 3000\nhealth = \"/healthz\"\n"
                ),
            )
            .unwrap();
        }
        let output = publish_output(
            PublishArgs {
                target: dir.join("gumgum.toml"),
                host: None,
                public_domain: None,
                tunnel: "byo".to_owned(),
            },
            &server_record(),
        )
        .unwrap();
        let PublishOutput::Workspace(report) = output else {
            panic!("expected workspace report")
        };
        assert_eq!(report.workers.len(), 2);
        assert_eq!(report.workers[0].worker, "api");
        assert_eq!(report.workers[1].worker, "worker");
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gumgum-publish-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn server_record() -> ServerRecord {
        ServerRecord {
            name: "starbase2".to_owned(),
            host: "192.168.0.3".to_owned(),
            root_domain: "leostera.dev".to_owned(),
            test_domain: "leostera.test".to_owned(),
            health_url: "http://starbase2:7777/healthz".to_owned(),
        }
    }
}
