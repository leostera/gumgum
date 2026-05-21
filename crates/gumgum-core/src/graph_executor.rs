use crate::{Capability, DesiredGraph, GraphReconcileAction, GraphReconciler};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "target")]
pub enum GraphExecutionTarget {
    Provider {
        name: String,
        capability: Capability,
    },
    WorkerRuntime {
        worker: String,
        image: String,
    },
    ContainerRuntime {
        container: String,
        image: String,
    },
    Gateway {
        host: String,
        target_container: String,
    },
    GraphStore {
        node: String,
    },
    Removal {
        id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GraphExecutionStep {
    pub action: GraphReconcileAction,
    pub target: GraphExecutionTarget,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GraphReconciliationPlan {
    pub actions: Vec<GraphReconcileAction>,
    pub steps: Vec<GraphExecutionStep>,
}

impl GraphReconciliationPlan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

pub struct GraphActionPlanner;

impl GraphActionPlanner {
    pub fn plan(actions: &[GraphReconcileAction]) -> Vec<GraphExecutionStep> {
        actions.iter().map(plan_action).collect()
    }

    pub fn plan_transition(
        old_graph: &DesiredGraph,
        new_graph: &DesiredGraph,
    ) -> GraphReconciliationPlan {
        let actions = GraphReconciler::reconcile(old_graph, new_graph);
        let steps = Self::plan(&actions);
        GraphReconciliationPlan { actions, steps }
    }
}

fn plan_action(action: &GraphReconcileAction) -> GraphExecutionStep {
    match action {
        GraphReconcileAction::EnsureProvider { name, capability } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Provider {
                name: name.clone(),
                capability: *capability,
            },
            description: format!("ensure provider {name} exists and is running"),
        },
        GraphReconcileAction::EnsureWorker { name, image } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::WorkerRuntime {
                worker: name.clone(),
                image: image.clone(),
            },
            description: format!("ensure worker {name} is represented in desired runtime state"),
        },
        GraphReconcileAction::EnsureContainer { name, image } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::ContainerRuntime {
                container: name.clone(),
                image: image.clone(),
            },
            description: format!("ensure container {name} runs image {image}"),
        },
        GraphReconcileAction::EnsureRoute {
            host,
            target_container,
        } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Gateway {
                host: host.clone(),
                target_container: target_container.clone(),
            },
            description: format!("ensure route {host} points at {target_container}"),
        },
        GraphReconcileAction::EnsureBinding {
            worker,
            name,
            object,
        } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::GraphStore {
                node: format!("binding/{worker}/{name}"),
            },
            description: format!("ensure binding {name} projects {object} into worker {worker}"),
        },
        GraphReconcileAction::EnsureObject {
            capability,
            name,
            provider,
        } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Provider {
                name: provider.clone(),
                capability: *capability,
            },
            description: format!("ensure {capability} object {name} is materialized by {provider}"),
        },
        GraphReconcileAction::RemoveNode { id } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Removal { id: id.clone() },
            description: format!("remove graph node {id}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_builds_full_transition_plan_from_graph_diff() {
        let old_graph = DesiredGraph::default();
        let new_graph = DesiredGraph::new([
            crate::DesiredGraphNode::Provider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            },
            crate::DesiredGraphNode::Object {
                capability: Capability::Secret,
                name: "stripe-api-key".to_owned(),
                provider: "vaultwarden.main".to_owned(),
            },
        ]);

        let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);

        assert_eq!(plan.actions.len(), 2);
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.steps.iter().any(|step| matches!(
            step.target,
            GraphExecutionTarget::Provider { ref name, capability: Capability::Secret }
                if name == "vaultwarden.main"
        )));
    }

    #[test]
    fn planner_routes_provider_actions_to_provider_executor_target() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureProvider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            },
            GraphReconcileAction::EnsureObject {
                capability: Capability::Blob,
                name: "uploads".to_owned(),
                provider: "minio.main".to_owned(),
            },
        ]);

        assert_eq!(
            steps[0].target,
            GraphExecutionTarget::Provider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            }
        );
        assert_eq!(
            steps[1].target,
            GraphExecutionTarget::Provider {
                name: "minio.main".to_owned(),
                capability: Capability::Blob,
            }
        );
    }

    #[test]
    fn planner_routes_runtime_and_gateway_actions_to_distinct_targets() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureContainer {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
            GraphReconcileAction::EnsureRoute {
                host: "api.example.test".to_owned(),
                target_container: "api".to_owned(),
            },
        ]);

        assert!(matches!(
            steps[0].target,
            GraphExecutionTarget::ContainerRuntime { .. }
        ));
        assert!(matches!(
            steps[1].target,
            GraphExecutionTarget::Gateway { .. }
        ));
    }
}
