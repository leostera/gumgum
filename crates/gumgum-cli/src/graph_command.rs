use crate::{GraphArgs, GraphCommand, print_value, resolve_server};
use gumgum_api::GraphReport;
use gumgum_core::{
    ErrorCode, GraphNode, GumgumError, ManifestKind, Subsystem, load_worker_path,
    load_workspace_path, validate_path,
};
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
                graph_affected(&server, &resolve_graph_target_arg(&target)?, json).await
            } else {
                graph_show(&server, json).await
            }
        }
        GraphCommand::Affected { target } => {
            graph_affected(&server, &resolve_graph_target_arg(&target)?, json).await
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

fn resolve_graph_target_arg(target: &str) -> gumgum_core::Result<String> {
    resolve_graph_target_arg_from_path(target, PathBuf::from("gumgum.toml"))
}

fn resolve_graph_target_arg_from_path(
    target: &str,
    manifest_path: PathBuf,
) -> gumgum_core::Result<String> {
    if target.contains('/') || target.contains('.') {
        return Ok(normalize_graph_target(target));
    }
    if manifest_path.exists() {
        match validate_path(&manifest_path)?.manifest_kind {
            ManifestKind::Worker => {
                let manifest = load_worker_path(&manifest_path)?;
                if target == manifest.worker.name {
                    return Ok(format!("worker/{}", manifest.worker.name));
                }
            }
            ManifestKind::Workspace => {
                let workspace = load_workspace_path(&manifest_path)?;
                let root = manifest_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                for member in &workspace.workspace.members {
                    let member_path = root.join(member).join("gumgum.toml");
                    let manifest = load_worker_path(&member_path)?;
                    if target == member || target == manifest.worker.name {
                        return Ok(format!("worker/{}", manifest.worker.name));
                    }
                }
            }
        }
    }
    Ok(normalize_graph_target(target))
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
    let client = ServerClient::new(server);
    let graph = client.graph().await?;
    validate_graph_target(target, &graph)?;
    let report = client.affected(target).await?;
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

fn validate_graph_target(target: &str, graph: &GraphReport) -> gumgum_core::Result<()> {
    if graph.nodes.iter().any(|node| node.id == target) {
        return Ok(());
    }
    let suggestion = suggest_graph_target(target, &graph.nodes);
    let mut builder = GumgumError::structured(
        Subsystem::Cli,
        ErrorCode::InvalidArgs,
        format!("unknown graph target: {target}"),
    )
    .likely_cause("target must be an existing desired graph node");
    if let Some(suggestion) = suggestion {
        builder = builder.next_command(format!("gumgum graph affected {suggestion}"));
    } else {
        builder = builder.next_command("gumgum graph show");
    }
    Err(builder.build())
}

fn suggest_graph_target(target: &str, nodes: &[GraphNode]) -> Option<String> {
    let leaf = target.rsplit('/').next().unwrap_or(target);
    nodes
        .iter()
        .filter(|node| node.id.starts_with("worker/") || node.id.starts_with("route/"))
        .find(|node| node.id.ends_with(leaf) || node.label.contains(leaf))
        .map(|node| node.id.clone())
        .or_else(|| {
            nodes
                .iter()
                .find(|node| node.id.contains(leaf) || node.label.contains(leaf))
                .map(|node| node.id.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_graph_target_maps_workspace_member_aliases() {
        let dir = std::env::temp_dir().join(format!(
            "gumgum-graph-target-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::write(
            dir.join("gumgum.toml"),
            "[workspace]\nname = \"visit-counter\"\nmembers = [\"api\"]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("api").join("gumgum.toml"),
            "[project]\nnamespace = \"visit-counter\"\n\n[worker]\nname = \"visit-counter-api\"\nbuild_context = \".\"\nport = 3000\nhealth = \"/healthz\"\n",
        )
        .unwrap();
        assert_eq!(
            resolve_graph_target_arg_from_path("api", dir.join("gumgum.toml")).unwrap(),
            "worker/visit-counter-api"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_graph_target_rejects_unknown_target_with_suggestion() {
        let graph = GraphReport {
            ok: true,
            format: "json".to_owned(),
            graph: String::new(),
            nodes: vec![GraphNode::new(
                "worker/visit-counter-api",
                "worker",
                "visit-counter-api",
            )],
            edges: Vec::new(),
        };

        let report = validate_graph_target("worker/api", &graph)
            .unwrap_err()
            .to_report();

        assert!(report.message.contains("unknown graph target"));
        assert!(
            report
                .next_commands
                .contains(&"gumgum graph affected worker/visit-counter-api".to_owned())
        );
    }

    #[test]
    fn validate_graph_target_accepts_existing_target() {
        let graph = GraphReport {
            ok: true,
            format: "json".to_owned(),
            graph: String::new(),
            nodes: vec![GraphNode::new("worker/api", "worker", "api")],
            edges: Vec::new(),
        };

        validate_graph_target("worker/api", &graph).unwrap();
    }
}
