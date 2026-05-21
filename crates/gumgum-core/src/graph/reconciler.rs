use crate::{
    BindingName, Capability, ContainerName, GraphNodeId, HealthPath, ImageName, ObjectName,
    ObjectRef, Port, ProviderName, RouteHost, WorkerId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DesiredGraphNode {
    Daemon {
        name: String,
    },
    Provider {
        name: ProviderName,
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
    Deployment {
        worker: WorkerId,
        image: ImageName,
        container: ContainerName,
        route: RouteHost,
        port: Port,
        health: HealthPath,
    },
    Route {
        host: String,
        target_container: String,
    },
    Binding {
        worker: WorkerId,
        name: BindingName,
        object: ObjectRef,
    },
    Object {
        capability: Capability,
        name: ObjectName,
        provider: ProviderName,
    },
}

impl DesiredGraphNode {
    pub fn id(&self) -> String {
        match self {
            Self::Daemon { name } => name.clone(),
            Self::Provider { name, .. } => format!("provider/{name}"),
            Self::Worker { name, .. } => format!("worker/{name}"),
            Self::Container { name, .. } => format!("container/{name}"),
            Self::Deployment { worker, .. } => format!("deployment/{worker}"),
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
        name: ProviderName,
        capability: Capability,
    },
    EnsureWorker {
        name: WorkerId,
        image: ImageName,
    },
    EnsureContainer {
        name: ContainerName,
        image: ImageName,
    },
    EnsureDeploy {
        worker: WorkerId,
        image: ImageName,
        container: ContainerName,
        route: RouteHost,
        port: Port,
        health: HealthPath,
    },
    EnsureRoute {
        host: RouteHost,
        target_container: ContainerName,
    },
    EnsureBinding {
        worker: WorkerId,
        name: BindingName,
        object: ObjectRef,
    },
    EnsureObject {
        capability: Capability,
        name: ObjectName,
        provider: ProviderName,
    },
    RemoveNode {
        id: GraphNodeId,
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
            actions.push(GraphReconcileAction::RemoveNode {
                id: GraphNodeId::new(node.id())
                    .unwrap_or_else(|_| GraphNodeId::new("unknown").unwrap()),
            });
        }
        actions
    }
}

fn ensure_action(node: &DesiredGraphNode) -> GraphReconcileAction {
    match node {
        DesiredGraphNode::Daemon { name } => GraphReconcileAction::EnsureProvider {
            name: ProviderName::new(name).unwrap_or_else(|_| ProviderName::new("daemon").unwrap()),
            capability: Capability::Manual,
        },
        DesiredGraphNode::Provider { name, capability } => GraphReconcileAction::EnsureProvider {
            name: name.clone(),
            capability: *capability,
        },
        DesiredGraphNode::Worker { name, image } => GraphReconcileAction::EnsureWorker {
            name: WorkerId::new(name).unwrap_or_else(|_| WorkerId::new("worker").unwrap()),
            image: ImageName::new(image)
                .unwrap_or_else(|_| ImageName::new("invalid:latest").unwrap()),
        },
        DesiredGraphNode::Container { name, image } => GraphReconcileAction::EnsureContainer {
            name: ContainerName::new(name)
                .unwrap_or_else(|_| ContainerName::new("container").unwrap()),
            image: ImageName::new(image)
                .unwrap_or_else(|_| ImageName::new("invalid:latest").unwrap()),
        },
        DesiredGraphNode::Deployment {
            worker,
            image,
            container,
            route,
            port,
            health,
        } => GraphReconcileAction::EnsureDeploy {
            worker: worker.clone(),
            image: image.clone(),
            container: container.clone(),
            route: route.clone(),
            port: *port,
            health: health.clone(),
        },
        DesiredGraphNode::Route {
            host,
            target_container,
        } => GraphReconcileAction::EnsureRoute {
            host: RouteHost::new(host).unwrap_or_else(|_| RouteHost::new("invalid.local").unwrap()),
            target_container: ContainerName::new(target_container)
                .unwrap_or_else(|_| ContainerName::new("container").unwrap()),
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
                name: ProviderName::new("vaultwarden.main").unwrap(),
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
            name: ProviderName::new("vaultwarden.main").unwrap(),
            capability: Capability::Secret
        }));
        assert!(actions.contains(&GraphReconcileAction::EnsureWorker {
            name: WorkerId::new("api").unwrap(),
            image: ImageName::new("ghcr.io/acme/api:v1").unwrap()
        }));
        assert!(actions.contains(&GraphReconcileAction::EnsureRoute {
            host: RouteHost::new("api.example.test").unwrap(),
            target_container: ContainerName::new("api").unwrap()
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
                id: GraphNodeId::new("container/api").unwrap()
            }]
        );
    }
}
