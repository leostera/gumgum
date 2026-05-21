use crate::{GraphEdge, GraphNode, PresentationGraph};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub action: String,
}

impl PlanNode {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        label: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            action: action.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

impl PlanEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PlanGraph {
    pub nodes: Vec<PlanNode>,
    pub edges: Vec<PlanEdge>,
    pub execution_levels: Vec<Vec<String>>,
}

impl PlanGraph {
    pub fn new(nodes: Vec<PlanNode>, edges: Vec<PlanEdge>) -> Self {
        let execution_levels = topological_levels(
            nodes.iter().map(|node| node.id.clone()),
            edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone())),
        );
        Self {
            nodes,
            edges,
            execution_levels,
        }
    }
}

impl PresentationGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_node(mut self, node: GraphNode) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn with_edge(mut self, edge: GraphEdge) -> Self {
        self.edges.push(edge);
        self
    }

    pub fn topological_levels(&self) -> Vec<Vec<String>> {
        topological_levels(
            self.nodes.iter().map(|node| node.id.clone()),
            self.edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone())),
        )
    }
}

fn topological_levels(
    nodes: impl IntoIterator<Item = String>,
    edges: impl IntoIterator<Item = (String, String)>,
) -> Vec<Vec<String>> {
    let edges = edges.into_iter().collect::<Vec<_>>();
    let mut remaining = nodes.into_iter().collect::<std::collections::BTreeSet<_>>();
    let mut levels = Vec::new();
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|id| {
                edges
                    .iter()
                    .all(|(from, to)| to != *id || !remaining.contains(from))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            levels.push(remaining.iter().cloned().collect());
            break;
        }
        for id in &ready {
            remaining.remove(id);
        }
        levels.push(ready);
    }
    levels
}
