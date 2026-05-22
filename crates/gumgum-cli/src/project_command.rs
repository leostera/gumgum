use crate::{InfoArgs, RollbackArgs, print_value, resolve_server};
use gumgum_api::{
    DeploymentRevisionDeleteReport, DeploymentRevisionsReport, GraphEdge, GraphNode, RollbackReport,
};
use gumgum_core::{
    ErrorCode, GumgumError, ManifestKind, Subsystem, load_worker_path, load_workspace_path,
    validate_path,
};
use serde::Serialize;

use crate::server_client::ServerClient;

#[derive(Debug, Serialize)]
struct InfoReport {
    ok: bool,
    worker: String,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    urls: Vec<String>,
    latest_image: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceInfoReport {
    ok: bool,
    workspace: String,
    workers: Vec<InfoReport>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum InfoOutput {
    Worker(InfoReport),
    Workspace(WorkspaceInfoReport),
}

pub(crate) async fn info(args: InfoArgs, json: bool) -> gumgum_core::Result<()> {
    let report = info_report(args).await?;
    if json {
        print_value(true, &report);
    } else {
        print_info_output(&report);
    }
    Ok(())
}

async fn info_report(args: InfoArgs) -> gumgum_core::Result<InfoOutput> {
    let kind = validate_path(&args.path)?.manifest_kind;
    let server = resolve_server(args.host)?;
    let client = ServerClient::new(server.host);
    match kind {
        ManifestKind::Worker => {
            let worker = args.worker.unwrap_or_else(|| {
                load_worker_path(&args.path)
                    .map(|manifest| manifest.worker.name)
                    .unwrap_or_else(|_| "unknown".to_owned())
            });
            Ok(InfoOutput::Worker(info_for_worker(&client, worker).await?))
        }
        ManifestKind::Workspace => {
            let workspace = load_workspace_path(&args.path)?;
            let root = args
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut workers = Vec::new();
            for member in &workspace.workspace.members {
                let member_path = root.join(member).join("gumgum.toml");
                let worker = load_worker_path(&member_path)?.worker.name;
                if args
                    .worker
                    .as_ref()
                    .is_none_or(|selected| selected == &worker)
                {
                    workers.push(info_for_worker(&client, worker).await?);
                }
            }
            Ok(InfoOutput::Workspace(WorkspaceInfoReport {
                ok: true,
                workspace: workspace.workspace.name,
                workers,
                message: "current workspace info".to_owned(),
            }))
        }
    }
}

async fn info_for_worker(client: &ServerClient, worker: String) -> gumgum_core::Result<InfoReport> {
    let target = format!("worker/{worker}");
    let affected = client.affected(&target).await?;
    let urls = affected
        .nodes
        .iter()
        .filter(|node| node.kind == "route")
        .map(|node| format!("http://{}", node.label))
        .collect::<Vec<_>>();
    let latest_image = affected
        .nodes
        .iter()
        .find(|node| node.kind == "image")
        .map(|node| node.label.clone());
    Ok(InfoReport {
        ok: true,
        worker,
        nodes: affected.nodes,
        edges: affected.edges,
        urls,
        latest_image,
        message: "current project info".to_owned(),
    })
}

fn print_info_output(report: &InfoOutput) {
    for line in info_lines(report) {
        println!("{line}");
    }
}

fn info_lines(report: &InfoOutput) -> Vec<String> {
    match report {
        InfoOutput::Worker(report) => info_report_lines(report),
        InfoOutput::Workspace(report) => {
            let mut lines = vec![format!("Workspace: {}", report.workspace)];
            for worker in &report.workers {
                lines.extend(info_report_lines(worker));
            }
            lines
        }
    }
}

fn info_report_lines(report: &InfoReport) -> Vec<String> {
    let mut lines = vec![format!("Worker: {}", report.worker)];
    lines.extend(report.urls.iter().map(|url| format!("URL: {url}")));
    if let Some(image) = &report.latest_image {
        lines.push(format!("Image: {image}"));
    }
    lines
}

pub(crate) async fn rollback(args: RollbackArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let client = ServerClient::new(server.host);
    if let Some(revision_id) = args.delete_revision_id {
        if args.preview || args.revisions || args.revision_id.is_some() {
            return Err(GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "--delete-revision-id cannot be combined with rollback preview, apply, or revision listing flags",
            )
            .next_command("gumgum rollback <path> --revisions")
            .next_command(format!(
                "gumgum rollback <path> --worker {worker} --delete-revision-id {revision_id}"
            ))
            .build());
        }
        let report = client.delete_revision(&worker, revision_id).await?;
        if json {
            print_value(true, &report);
        } else {
            for line in revision_delete_lines(&report) {
                println!("{line}");
            }
        }
        return Ok(());
    }
    if args.revisions {
        let report = client.revisions(&worker, args.limit).await?;
        if json {
            print_value(true, &report);
        } else {
            for line in revision_lines(&report) {
                println!("{line}");
            }
        }
        return Ok(());
    }
    let report = client
        .rollback(worker, args.preview, args.revision_id)
        .await?;
    if json {
        print_value(true, &report);
    } else {
        for line in rollback_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn revision_delete_lines(report: &DeploymentRevisionDeleteReport) -> Vec<String> {
    let mut lines = vec![if report.deleted {
        format!(
            "Deleted deployment revision #{} for {}",
            report.revision_id, report.worker
        )
    } else {
        format!(
            "Deployment revision #{} for {} was not found",
            report.revision_id, report.worker
        )
    }];
    if !report.actions.is_empty() {
        lines.push("Actions:".to_owned());
        lines.extend(report.actions.iter().map(|action| format!("  - {action}")));
    }
    lines
}

fn revision_lines(report: &DeploymentRevisionsReport) -> Vec<String> {
    let mut lines = vec![format!(
        "Deployment revisions for {} ({}):",
        report.worker,
        report.revisions.len()
    )];
    if let Some(current) = &report.current {
        lines.push(format!(
            "Current: image={} route={} container={} port={} health={}",
            current.image, current.route, current.container, current.port, current.health
        ));
    }
    for revision in &report.revisions {
        lines.push(format!(
            "#{} {} image={} route={} container={} port={} health={}",
            revision.id,
            revision.created_at,
            revision.deploy.image,
            revision.deploy.route,
            revision.deploy.container,
            revision.deploy.port,
            revision.deploy.health
        ));
        if let Some(current) = &report.current {
            if revision.deploy.route != current.route {
                lines.push(format!(
                    "  warning: rollback would change route from {} to {}",
                    current.route, revision.deploy.route
                ));
            }
        }
        lines.push(format!(
            "  preview: gumgum rollback --worker {} --revision-id {} --preview",
            report.worker, revision.id
        ));
        lines.push(format!(
            "  apply:   gumgum rollback --worker {} --revision-id {}",
            report.worker, revision.id
        ));
    }
    lines
}

fn rollback_lines(report: &RollbackReport) -> Vec<String> {
    if !report.ok {
        return vec![format!(
            "Rollback unavailable for {}: {}",
            report.worker, report.message
        )];
    }
    let mut lines = vec![if report.message == "rollback preview" {
        format!("Rollback preview for worker {}", report.worker)
    } else {
        format!("Rolled back worker {}", report.worker)
    }];
    if let Some(revision_id) = report.revision_id {
        lines.push(format!("Revision: {revision_id}"));
    }
    if let Some(image) = &report.image {
        lines.push(format!("Image: {image}"));
    }
    if let Some(container) = &report.container {
        lines.push(format!("Container: {container}"));
    }
    if let Some(route) = &report.route {
        lines.push(format!("Route: {route}"));
    }
    if let Some(port) = report.port {
        lines.push(format!("Port: {port}"));
    }
    if let Some(health) = &report.health {
        lines.push(format!("Health: {health}"));
    }
    if !report.actions.is_empty() {
        lines.push("Actions:".to_owned());
        lines.extend(report.actions.iter().map(|action| format!("  - {action}")));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_lines_include_full_target() {
        let report = RollbackReport {
            ok: true,
            worker: "api".to_owned(),
            image: Some("registry/api:1".to_owned()),
            revision_id: Some(42),
            container: Some("gumgum-api".to_owned()),
            route: Some("api.example.test".to_owned()),
            port: Some(3000),
            health: Some("/healthz".to_owned()),
            actions: vec!["rollback to registry/api:1".to_owned()],
            message: "rollback applied".to_owned(),
        };

        assert_eq!(
            rollback_lines(&report),
            vec![
                "Rolled back worker api",
                "Revision: 42",
                "Image: registry/api:1",
                "Container: gumgum-api",
                "Route: api.example.test",
                "Port: 3000",
                "Health: /healthz",
                "Actions:",
                "  - rollback to registry/api:1",
            ]
        );
    }

    #[test]
    fn revision_delete_lines_explain_safe_history_prune() {
        let report = DeploymentRevisionDeleteReport {
            ok: true,
            worker: "api".to_owned(),
            revision_id: 8,
            deleted: true,
            actions: vec![
                "deleted deployment revision 8".to_owned(),
                "no containers or desired deployments changed".to_owned(),
            ],
            message: "deleted deployment revision 8".to_owned(),
        };

        assert_eq!(
            revision_delete_lines(&report),
            vec![
                "Deleted deployment revision #8 for api",
                "Actions:",
                "  - deleted deployment revision 8",
                "  - no containers or desired deployments changed",
            ]
        );
    }

    #[test]
    fn revision_lines_include_revision_metadata() {
        let report = DeploymentRevisionsReport {
            ok: true,
            worker: "api".to_owned(),
            current: Some(gumgum_core::DesiredDeploy {
                worker: "api".to_owned(),
                image: "registry/api:2".to_owned(),
                container: "gumgum-api".to_owned(),
                route: "api.current.test".to_owned(),
                port: 3000,
                health: "/healthz".to_owned(),
            }),
            revisions: vec![gumgum_core::DeploymentRevision {
                id: 42,
                created_at: "2026-05-20 12:00:00".to_owned(),
                deploy: gumgum_core::DesiredDeploy {
                    worker: "api".to_owned(),
                    image: "registry/api:1".to_owned(),
                    container: "gumgum-api".to_owned(),
                    route: "api.example.test".to_owned(),
                    port: 3000,
                    health: "/healthz".to_owned(),
                },
            }],
            message: "1 deployment revision(s)".to_owned(),
        };

        assert_eq!(
            revision_lines(&report),
            vec![
                "Deployment revisions for api (1):",
                "Current: image=registry/api:2 route=api.current.test container=gumgum-api port=3000 health=/healthz",
                "#42 2026-05-20 12:00:00 image=registry/api:1 route=api.example.test container=gumgum-api port=3000 health=/healthz",
                "  warning: rollback would change route from api.current.test to api.example.test",
                "  preview: gumgum rollback --worker api --revision-id 42 --preview",
                "  apply:   gumgum rollback --worker api --revision-id 42",
            ]
        );
    }

    #[test]
    fn rollback_lines_include_preview_heading() {
        let report = RollbackReport {
            ok: true,
            worker: "api".to_owned(),
            image: Some("registry/api:1".to_owned()),
            revision_id: Some(42),
            container: None,
            route: None,
            port: None,
            health: None,
            actions: vec!["preview only; no containers changed".to_owned()],
            message: "rollback preview".to_owned(),
        };

        let lines = rollback_lines(&report);
        assert_eq!(lines[0], "Rollback preview for worker api");
        assert!(lines.contains(&"  - preview only; no containers changed".to_owned()));
    }

    #[test]
    fn rollback_lines_explain_unavailable_state() {
        let report = RollbackReport {
            ok: false,
            worker: "api".to_owned(),
            image: None,
            revision_id: None,
            container: None,
            route: None,
            port: None,
            health: None,
            actions: vec!["no previous image recorded".to_owned()],
            message: "no previous deployment image recorded".to_owned(),
        };

        assert_eq!(
            rollback_lines(&report),
            vec!["Rollback unavailable for api: no previous deployment image recorded"]
        );
    }

    #[test]
    fn workspace_info_lines_include_all_members() {
        let report = InfoOutput::Workspace(WorkspaceInfoReport {
            ok: true,
            workspace: "visit-counter".to_owned(),
            workers: vec![
                InfoReport {
                    ok: true,
                    worker: "api".to_owned(),
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    urls: vec!["http://api.example.test".to_owned()],
                    latest_image: Some("registry/api:1".to_owned()),
                    message: "current project info".to_owned(),
                },
                InfoReport {
                    ok: true,
                    worker: "worker".to_owned(),
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    urls: Vec::new(),
                    latest_image: None,
                    message: "current project info".to_owned(),
                },
            ],
            message: "current workspace info".to_owned(),
        });

        assert_eq!(
            info_lines(&report),
            vec![
                "Workspace: visit-counter",
                "Worker: api",
                "URL: http://api.example.test",
                "Image: registry/api:1",
                "Worker: worker",
            ]
        );
    }
}
