use crate::Capability;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DesiredGraphNode {
    Daemon {
        name: String,
    },
    Provider {
        name: String,
        capability: Capability,
    },
    Worker {
        name: String,
        image: String,
    },
    Container {
        name: String,
        image: String,
    },
    Route {
        host: String,
        target_container: String,
    },
    Binding {
        worker: String,
        name: String,
        object: String,
    },
    Object {
        capability: Capability,
        name: String,
        provider: String,
    },
}

impl DesiredGraphNode {
    pub fn id(&self) -> String {
        match self {
            Self::Daemon { name } => name.clone(),
            Self::Provider { name, .. } => format!("provider/{name}"),
            Self::Worker { name, .. } => format!("worker/{name}"),
            Self::Container { name, .. } => format!("container/{name}"),
            Self::Route { host, .. } => format!("route/{host}"),
            Self::Binding { worker, name, .. } => format!("binding/{worker}/{name}"),
            Self::Object {
                capability, name, ..
            } => format!("{capability}/{name}"),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct DesiredGraph {
    pub nodes: BTreeSet<DesiredGraphNode>,
}

impl DesiredGraph {
    pub fn new(nodes: impl IntoIterator<Item = DesiredGraphNode>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum GraphReconcileAction {
    EnsureProvider {
        name: String,
        capability: Capability,
    },
    EnsureWorker {
        name: String,
        image: String,
    },
    EnsureContainer {
        name: String,
        image: String,
    },
    EnsureRoute {
        host: String,
        target_container: String,
    },
    EnsureBinding {
        worker: String,
        name: String,
        object: String,
    },
    EnsureObject {
        capability: Capability,
        name: String,
        provider: String,
    },
    RemoveNode {
        id: String,
    },
}

pub struct GraphReconciler;

impl GraphReconciler {
    pub fn reconcile(
        old_graph: &DesiredGraph,
        new_graph: &DesiredGraph,
    ) -> Vec<GraphReconcileAction> {
        let mut actions = Vec::new();
        for node in new_graph.nodes.difference(&old_graph.nodes) {
            actions.push(ensure_action(node));
        }
        for node in old_graph.nodes.difference(&new_graph.nodes) {
            actions.push(GraphReconcileAction::RemoveNode { id: node.id() });
        }
        actions
    }
}

fn ensure_action(node: &DesiredGraphNode) -> GraphReconcileAction {
    match node {
        DesiredGraphNode::Daemon { name } => GraphReconcileAction::EnsureProvider {
            name: name.clone(),
            capability: Capability::Manual,
        },
        DesiredGraphNode::Provider { name, capability } => GraphReconcileAction::EnsureProvider {
            name: name.clone(),
            capability: *capability,
        },
        DesiredGraphNode::Worker { name, image } => GraphReconcileAction::EnsureWorker {
            name: name.clone(),
            image: image.clone(),
        },
        DesiredGraphNode::Container { name, image } => GraphReconcileAction::EnsureContainer {
            name: name.clone(),
            image: image.clone(),
        },
        DesiredGraphNode::Route {
            host,
            target_container,
        } => GraphReconcileAction::EnsureRoute {
            host: host.clone(),
            target_container: target_container.clone(),
        },
        DesiredGraphNode::Binding {
            worker,
            name,
            object,
        } => GraphReconcileAction::EnsureBinding {
            worker: worker.clone(),
            name: name.clone(),
            object: object.clone(),
        },
        DesiredGraphNode::Object {
            capability,
            name,
            provider,
        } => GraphReconcileAction::EnsureObject {
            capability: *capability,
            name: name.clone(),
            provider: provider.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_reconciler_emits_typed_actions_for_new_nodes() {
        let old_graph = DesiredGraph::default();
        let new_graph = DesiredGraph::new([
            DesiredGraphNode::Provider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            },
            DesiredGraphNode::Worker {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
            DesiredGraphNode::Route {
                host: "api.example.test".to_owned(),
                target_container: "api".to_owned(),
            },
        ]);

        let actions = GraphReconciler::reconcile(&old_graph, &new_graph);

        assert_eq!(actions.len(), 3);
        assert!(actions.contains(&GraphReconcileAction::EnsureProvider {
            name: "vaultwarden.main".to_owned(),
            capability: Capability::Secret
        }));
        assert!(actions.contains(&GraphReconcileAction::EnsureWorker {
            name: "api".to_owned(),
            image: "ghcr.io/acme/api:v1".to_owned()
        }));
        assert!(actions.contains(&GraphReconcileAction::EnsureRoute {
            host: "api.example.test".to_owned(),
            target_container: "api".to_owned()
        }));
    }

    #[test]
    fn graph_reconciler_removes_nodes_absent_from_next_graph() {
        let old_graph = DesiredGraph::new([DesiredGraphNode::Container {
            name: "api".to_owned(),
            image: "old".to_owned(),
        }]);
        let new_graph = DesiredGraph::default();

        assert_eq!(
            GraphReconciler::reconcile(&old_graph, &new_graph),
            vec![GraphReconcileAction::RemoveNode {
                id: "container/api".to_owned()
            }]
        );
    }
}
