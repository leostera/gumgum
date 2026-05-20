use crate::{InfoArgs, RollbackArgs, print_value, resolve_server};
use gumgum_api::{GraphEdge, GraphNode};
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
    print_value(json, &report);
    Ok(())
}
