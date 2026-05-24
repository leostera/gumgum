use crate::{GraphEdge, GraphNode, GraphProjection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub action: PlanAction,
}

impl PlanNode {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        label: impl Into<String>,
        action: PlanAction,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            action,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    CollectManifestDesiredState,
    CollectActualContainerState,
    EnsureLocalRegistryProvider,
    BuildAndPushWorkerImage,
    ReconcileWorkerContainer,
    VerifyHealthCheckAndRoutes,
    EnsureProviderRunning,
    EnsureGlobalObjectExists,
    EnsureWorkerLocalBindingExists,
    ReadDeployedWorker,
    PlanRouteMapping,
    PlanTunnelMapping,
    PreserveLocalRoute,
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

impl GraphProjection {
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
    let mut indegree = std::collections::BTreeMap::<String, usize>::new();
    let mut outgoing = std::collections::BTreeMap::<String, Vec<String>>::new();
    for node in nodes {
        indegree.entry(node).or_insert(0);
    }
    for (from, to) in edges {
        outgoing.entry(from.clone()).or_default().push(to.clone());
        *indegree.entry(to).or_insert(0) += 1;
        indegree.entry(from).or_insert(0);
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect::<Vec<_>>();
    let mut levels = Vec::new();
    while !ready.is_empty() {
        ready.sort();
        let level = ready;
        let mut next = Vec::new();
        for node in &level {
            if let Some(children) = outgoing.get(node) {
                for child in children {
                    if let Some(degree) = indegree.get_mut(child) {
                        *degree -= 1;
                        if *degree == 0 {
                            next.push(child.clone());
                        }
                    }
                }
            }
        }
        levels.push(level);
        ready = next;
    }
    levels
}
