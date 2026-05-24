use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphProjectionNode {
    pub id: String,
    pub kind: String,
    pub label: String,
}

impl GraphProjectionNode {
    pub fn new(id: impl Into<String>, kind: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphProjectionEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

impl GraphProjectionEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GraphProjection {
    pub nodes: Vec<GraphProjectionNode>,
    pub edges: Vec<GraphProjectionEdge>,
}

pub type Graph = GraphProjection;
pub type GraphNode = GraphProjectionNode;
pub type GraphEdge = GraphProjectionEdge;

pub fn affected_subgraph(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
    target: &str,
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut seen = std::collections::BTreeSet::new();
    let mut edge_seen = std::collections::BTreeSet::new();
    seen.insert(target.to_owned());

    let mut add_edge = |edge: &GraphEdge, seen: &mut std::collections::BTreeSet<String>| {
        edge_seen.insert((edge.from.clone(), edge.to.clone(), edge.kind.clone()));
        seen.insert(edge.from.clone());
        seen.insert(edge.to.clone());
    };

    for edge in edges {
        if edge.to == target || edge.from == target {
            add_edge(edge, &mut seen);
        }
    }

    let bindings = seen
        .iter()
        .filter(|id| id.starts_with("binding/"))
        .cloned()
        .collect::<Vec<_>>();
    for binding in bindings {
        for edge in edges {
            if edge.to == binding || edge.from == binding {
                add_edge(edge, &mut seen);
            }
        }
    }

    let workers = seen
        .iter()
        .filter(|id| id.starts_with("worker/"))
        .cloned()
        .collect::<Vec<_>>();
    for worker in workers {
        for edge in edges {
            if edge.from == worker && matches!(edge.kind.as_str(), "runs" | "owns" | "created_from")
            {
                add_edge(edge, &mut seen);
            }
        }
    }

    let routes = seen
        .iter()
        .filter(|id| id.starts_with("route/"))
        .cloned()
        .collect::<Vec<_>>();
    for route in routes {
        for edge in edges {
            if edge.from == route && edge.kind == "routes_to" {
                add_edge(edge, &mut seen);
            }
        }
    }

    let affected_nodes = nodes
        .iter()
        .filter(|node| seen.contains(&node.id))
        .cloned()
        .collect::<Vec<_>>();
    let affected_edges = edges
        .iter()
        .filter(|edge| edge_seen.contains(&(edge.from.clone(), edge.to.clone(), edge.kind.clone())))
        .cloned()
        .collect::<Vec<_>>();
    (affected_nodes, affected_edges)
}

#[cfg(test)]
mod graph_tests {
    use super::*;

    fn ids(nodes: &[GraphNode]) -> Vec<String> {
        nodes.iter().map(|node| node.id.clone()).collect()
    }

    #[test]
    fn affected_subgraph_expands_bindings_to_workers_and_runtime_nodes() {
        let nodes = vec![
            GraphNode::new("db/main", "global_object", "main"),
            GraphNode::new("binding/api/DATABASE_URL", "binding", "DATABASE_URL"),
            GraphNode::new("worker/api", "worker", "api"),
            GraphNode::new("container/api", "container", "api"),
            GraphNode::new("route/api.example.test", "route", "api.example.test"),
            GraphNode::new("unrelated", "worker", "unrelated"),
        ];
        let edges = vec![
            GraphEdge::new("db/main", "binding/api/DATABASE_URL", "projects_as"),
            GraphEdge::new("binding/api/DATABASE_URL", "worker/api", "injects_into"),
            GraphEdge::new("worker/api", "container/api", "runs"),
            GraphEdge::new("worker/api", "route/api.example.test", "owns"),
            GraphEdge::new("route/api.example.test", "container/api", "routes_to"),
            GraphEdge::new("unrelated", "container/other", "runs"),
        ];

        let (affected_nodes, affected_edges) = affected_subgraph(&nodes, &edges, "db/main");
        assert_eq!(
            ids(&affected_nodes),
            vec![
                "db/main",
                "binding/api/DATABASE_URL",
                "worker/api",
                "container/api",
                "route/api.example.test",
            ]
        );
        assert_eq!(affected_edges.len(), 5);
    }

    #[test]
    fn affected_subgraph_keeps_unknown_targets_empty_except_seen_id() {
        let nodes = vec![GraphNode::new("worker/api", "worker", "api")];
        let edges = Vec::new();
        let (affected_nodes, affected_edges) = affected_subgraph(&nodes, &edges, "missing");
        assert!(affected_nodes.is_empty());
        assert!(affected_edges.is_empty());
    }
}
