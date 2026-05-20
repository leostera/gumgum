use crate::{GraphArgs, GraphCommand, print_value, resolve_server};
use gumgum_core::load_worker_path;
use std::path::PathBuf;

use crate::{graph_presenter::GraphPresenter, server_client::ServerClient};

pub(crate) async fn graph(args: GraphArgs, json: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?.host;
    let scoped_target = args
        .resource
        .or_else(|| args.worker.map(|worker| format!("worker/{worker}")))
        .or_else(|| infer_graph_target_from_manifest().ok().flatten());
    match args.command.unwrap_or(GraphCommand::Show) {
        GraphCommand::Show => {
            if let Some(target) = scoped_target {
                graph_affected(&server, &normalize_graph_target(&target), json).await
            } else {
                graph_show(&server, json).await
            }
        }
        GraphCommand::Affected { target } => {
            graph_affected(&server, &normalize_graph_target(&target), json).await
        }
    }
}

fn infer_graph_target_from_manifest() -> gumgum_core::Result<Option<String>> {
    let path = PathBuf::from("gumgum.toml");
    if path.exists() {
        return Ok(Some(format!(
            "worker/{}",
            load_worker_path(&path)?.worker.name
        )));
    }
    Ok(None)
}

fn normalize_graph_target(target: &str) -> String {
    if target.contains('/') {
        target.to_owned()
    } else if target.contains('.') {
        format!("route/{target}")
    } else {
        format!("worker/{target}")
    }
}

async fn graph_show(server: &str, json: bool) -> gumgum_core::Result<()> {
    let report = ServerClient::new(server).graph().await?;
    if json {
        print_value(true, &report);
    } else {
        println!("{}", report.graph);
    }
    Ok(())
}

async fn graph_affected(server: &str, target: &str, json: bool) -> gumgum_core::Result<()> {
    let report = ServerClient::new(server).affected(target).await?;
    if json {
        print_value(true, &report);
    } else {
        println!("Affected by {}:", report.target);
        let presenter = GraphPresenter::new();
        for node in report.nodes {
            println!("  {}", presenter.describe_node(&node));
        }
    }
    Ok(())
}
