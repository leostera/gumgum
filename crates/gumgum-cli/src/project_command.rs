use crate::{InfoArgs, RollbackArgs, print_value, resolve_server};
use gumgum_api::{GraphEdge, GraphNode, RollbackReport};
use gumgum_core::load_worker_path;
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

pub(crate) async fn info(args: InfoArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let target = format!("worker/{worker}");
    let affected = ServerClient::new(server.host).affected(&target).await?;
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
    let report = InfoReport {
        ok: true,
        worker,
        nodes: affected.nodes,
        edges: affected.edges,
        urls,
        latest_image,
        message: "current project info".to_owned(),
    };
    if json {
        print_value(true, &report);
    } else {
        println!("Worker: {}", report.worker);
        for url in &report.urls {
            println!("URL: {url}");
        }
        if let Some(image) = &report.latest_image {
            println!("Image: {image}");
        }
    }
    Ok(())
}

pub(crate) async fn rollback(args: RollbackArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host).rollback(worker).await?;
    if json {
        print_value(true, &report);
    } else {
        for line in rollback_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn rollback_lines(report: &RollbackReport) -> Vec<String> {
    if !report.ok {
        return vec![format!(
            "Rollback unavailable for {}: {}",
            report.worker, report.message
        )];
    }
    let mut lines = vec![format!("Rolled back worker {}", report.worker)];
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
    fn rollback_lines_explain_unavailable_state() {
        let report = RollbackReport {
            ok: false,
            worker: "api".to_owned(),
            image: None,
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
}
