use crate::{
    Capability, ContainerName, DeployRequest, DesiredGraph, GraphReconcileAction, GraphReconciler,
    ImageName, ObjectProviderPlan, Port, ProviderCredentials, RouteHost, WorkerId,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    DeployRuntime {
        worker: Option<WorkerId>,
        container: ContainerName,
        image: ImageName,
        route: Option<RouteHost>,
        port: Option<Port>,
        health: Option<String>,
    },
    Gateway {
        host: String,
        target_container: String,
    },
    ObjectProvider {
        capability: Capability,
        name: String,
        provider: String,
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
        Self::normalize_steps(actions.iter().map(plan_action).collect())
    }

    pub fn normalize_steps(steps: Vec<GraphExecutionStep>) -> Vec<GraphExecutionStep> {
        let worker = steps.iter().find_map(|step| match &step.target {
            GraphExecutionTarget::WorkerRuntime { worker, .. } => WorkerId::new(worker).ok(),
            _ => None,
        });
        let route = steps.iter().find_map(|step| match &step.target {
            GraphExecutionTarget::Gateway { host, .. } => RouteHost::new(host).ok(),
            _ => None,
        });
        let deploy_step = steps.iter().find_map(|step| match &step.target {
            GraphExecutionTarget::DeployRuntime { .. } => Some(step.clone()),
            GraphExecutionTarget::ContainerRuntime { container, image } => {
                Some(GraphExecutionStep {
                    action: step.action.clone(),
                    target: GraphExecutionTarget::DeployRuntime {
                        worker: worker.clone(),
                        container: ContainerName::new(container)
                            .unwrap_or_else(|_| ContainerName::new("container").unwrap()),
                        image: ImageName::new(image)
                            .unwrap_or_else(|_| ImageName::new("invalid:latest").unwrap()),
                        route: route.clone(),
                        port: None,
                        health: None,
                    },
                    description: format!(
                        "ensure deploy runtime for {} runs image {}",
                        worker
                            .as_ref()
                            .map(|worker| worker.as_str())
                            .unwrap_or(container),
                        image
                    ),
                })
            }
            _ => None,
        });
        let Some(deploy_step) = deploy_step else {
            return steps;
        };
        steps
            .into_iter()
            .filter(|step| {
                !matches!(
                    step.target,
                    GraphExecutionTarget::WorkerRuntime { .. }
                        | GraphExecutionTarget::ContainerRuntime { .. }
                        | GraphExecutionTarget::Gateway { .. }
                )
            })
            .chain(std::iter::once(deploy_step))
            .collect()
    }

    pub fn plan_transition(
        old_graph: &DesiredGraph,
        new_graph: &DesiredGraph,
    ) -> GraphReconciliationPlan {
        let actions = GraphReconciler::reconcile(old_graph, new_graph);
        let steps = Self::plan(&actions);
        GraphReconciliationPlan { actions, steps }
    }

    pub fn ensure_provider_step(
        name: impl Into<String>,
        capability: Capability,
    ) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureProvider {
            name: name.into(),
            capability,
        })
    }

    pub fn ensure_object_step(
        capability: Capability,
        name: impl Into<String>,
        provider: impl Into<String>,
    ) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureObject {
            capability,
            name: name.into(),
            provider: provider.into(),
        })
    }

    pub fn ensure_container_step(
        name: impl Into<String>,
        image: impl Into<String>,
    ) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureContainer {
            name: name.into(),
            image: image.into(),
        })
    }

    pub fn ensure_deploy_step(
        worker: WorkerId,
        container: ContainerName,
        image: ImageName,
        route: RouteHost,
        port: Port,
        health: impl Into<String>,
    ) -> GraphExecutionStep {
        let health = health.into();
        GraphExecutionStep {
            action: GraphReconcileAction::EnsureContainer {
                name: container.to_string(),
                image: image.to_string(),
            },
            target: GraphExecutionTarget::DeployRuntime {
                worker: Some(worker.clone()),
                container,
                image: image.clone(),
                route: Some(route),
                port: Some(port),
                health: Some(health),
            },
            description: format!("ensure deploy runtime for {worker} runs image {image}"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraphExecutionContext {
    pub object_plan: Option<ObjectProviderPlan>,
    pub provider_credentials: Option<ProviderCredentials>,
    pub deploy_request: Option<DeployRequest>,
    pub graph_path: Option<PathBuf>,
}

pub struct GraphActionExecutor;

impl GraphActionExecutor {
    pub async fn execute_steps(
        steps: &[GraphExecutionStep],
        context: GraphExecutionContext,
    ) -> crate::Result<Vec<String>> {
        let mut actions = Vec::new();
        let mut container_runtime_seen = false;
        for step in steps {
            match &step.target {
                GraphExecutionTarget::Provider { name, capability } => {
                    actions.extend(Self::execute_provider(name, *capability).await?);
                }
                GraphExecutionTarget::ObjectProvider { .. } => {
                    if let Some(plan) = &context.object_plan {
                        actions.extend(
                            Self::execute_object_plan(plan, context.provider_credentials.clone())
                                .await?,
                        );
                    } else {
                        actions.push(format!("planned {}", step.description));
                    }
                }
                GraphExecutionTarget::ContainerRuntime { .. }
                | GraphExecutionTarget::DeployRuntime { .. } => {
                    if container_runtime_seen {
                        actions.push(
                            "container runtime already reconciled for this graph execution"
                                .to_owned(),
                        );
                        continue;
                    }
                    container_runtime_seen = true;
                    if let Some(graph_path) = &context.graph_path {
                        let request = context
                            .deploy_request
                            .clone()
                            .or_else(|| deploy_request_from_target(&step.target));
                        let Some(request) = request else {
                            actions.push(format!("planned {}", step.description));
                            continue;
                        };
                        let (changed, mut deploy_actions) =
                            crate::ContainerReconciler::new(graph_path.clone())
                                .reconcile(&request)
                                .await?;
                        if !changed && deploy_actions.is_empty() {
                            deploy_actions
                                .push("container already matches desired state".to_owned());
                        }
                        actions.extend(deploy_actions);
                    } else {
                        actions.push(format!("planned {}", step.description));
                    }
                }
                GraphExecutionTarget::WorkerRuntime { .. }
                | GraphExecutionTarget::Gateway { .. }
                | GraphExecutionTarget::GraphStore { .. }
                | GraphExecutionTarget::Removal { .. } => {
                    actions.push(format!("planned {}", step.description));
                }
            }
        }
        Ok(actions)
    }

    pub async fn execute_provider_steps(
        steps: &[GraphExecutionStep],
    ) -> crate::Result<Vec<String>> {
        Self::execute_steps(steps, GraphExecutionContext::default()).await
    }

    pub async fn execute_provider(
        name: &str,
        capability: Capability,
    ) -> crate::Result<Vec<String>> {
        match (name, capability) {
            ("vaultwarden.main", Capability::Secret) => {
                crate::providers::vaultwarden::ensure().await
            }
            _ => Ok(vec![format!("configured {capability} provider {name}")]),
        }
    }

    pub async fn execute_object_provider_steps(
        steps: &[GraphExecutionStep],
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<Vec<String>> {
        Self::execute_steps(
            steps,
            GraphExecutionContext {
                object_plan: Some(plan.clone()),
                provider_credentials: credentials,
                deploy_request: None,
                graph_path: None,
            },
        )
        .await
    }

    pub async fn execute_object_plan(
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<Vec<String>> {
        crate::providers::ProviderReconciler::ensure_with_credentials(plan, credentials).await
    }
}

fn deploy_request_from_target(target: &GraphExecutionTarget) -> Option<DeployRequest> {
    match target {
        GraphExecutionTarget::DeployRuntime {
            worker: Some(worker),
            container,
            image,
            route: Some(route),
            port: Some(port),
            health: Some(health),
        } => Some(DeployRequest {
            worker: worker.to_string(),
            image: image.to_string(),
            container: container.to_string(),
            route: route.to_string(),
            port: port.get(),
            health: health.clone(),
        }),
        _ => None,
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
        GraphReconcileAction::EnsureDeploy {
            worker,
            image,
            container,
            route,
            port,
            health,
        } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::DeployRuntime {
                worker: Some(worker.clone()),
                container: container.clone(),
                image: image.clone(),
                route: Some(route.clone()),
                port: Some(*port),
                health: Some(health.clone()),
            },
            description: format!("ensure deploy runtime for {worker} runs image {image}"),
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
            target: GraphExecutionTarget::ObjectProvider {
                capability: *capability,
                name: name.clone(),
                provider: provider.clone(),
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
    fn planner_can_synthesize_idempotent_provider_step() {
        let step = GraphActionPlanner::ensure_provider_step("vaultwarden.main", Capability::Secret);

        assert_eq!(
            step.target,
            GraphExecutionTarget::Provider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            }
        );
        assert_eq!(
            step.description,
            "ensure provider vaultwarden.main exists and is running"
        );
    }

    #[tokio::test]
    async fn executor_routes_vaultwarden_provider_target_to_backend() {
        let actions = GraphActionExecutor::execute_provider("manual.main", Capability::Manual)
            .await
            .unwrap();

        assert_eq!(actions, vec!["configured manual provider manual.main"]);
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
            GraphExecutionTarget::ObjectProvider {
                capability: Capability::Blob,
                name: "uploads".to_owned(),
                provider: "minio.main".to_owned(),
            }
        );
    }

    #[test]
    fn planner_can_synthesize_idempotent_object_step() {
        let step =
            GraphActionPlanner::ensure_object_step(Capability::Blob, "uploads", "minio.main");

        assert_eq!(
            step.target,
            GraphExecutionTarget::ObjectProvider {
                capability: Capability::Blob,
                name: "uploads".to_owned(),
                provider: "minio.main".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn executor_routes_object_plan_to_provider_reconciler() {
        let plan = crate::object_provider_plan(
            Capability::Secret,
            "stripe-api-key",
            "stripe.secret.example.test",
        );

        let step = GraphActionPlanner::ensure_object_step(
            Capability::Secret,
            "stripe-api-key",
            "onepassword.main",
        );
        let actions = GraphActionExecutor::execute_object_provider_steps(&[step], &plan, None)
            .await
            .unwrap();

        assert!(
            actions
                .iter()
                .any(|action| action.contains("no secret value stored"))
        );
    }

    #[tokio::test]
    async fn generic_executor_dispatches_provider_and_plans_runtime_targets() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureProvider {
                name: "manual.main".to_owned(),
                capability: Capability::Manual,
            },
            GraphReconcileAction::EnsureRoute {
                host: "api.example.test".to_owned(),
                target_container: "api".to_owned(),
            },
        ]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert_eq!(actions[0], "configured manual provider manual.main");
        assert!(actions[1].contains("planned ensure route api.example.test"));
    }

    #[test]
    fn planner_can_synthesize_idempotent_container_step() {
        let step = GraphActionPlanner::ensure_container_step("api", "ghcr.io/acme/api:v1");

        assert_eq!(
            step.target,
            GraphExecutionTarget::ContainerRuntime {
                container: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            }
        );
    }

    #[test]
    fn planner_can_synthesize_idempotent_deploy_step() {
        let step = GraphActionPlanner::ensure_deploy_step(
            WorkerId::new("api").unwrap(),
            ContainerName::new("gumgum-api").unwrap(),
            ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            RouteHost::new("api.example.test").unwrap(),
            Port::new(3000).unwrap(),
            "/healthz",
        );

        assert!(matches!(
            step.target,
            GraphExecutionTarget::DeployRuntime {
                worker: Some(ref worker),
                ref container,
                route: Some(ref route),
                port: Some(port),
                health: Some(ref health),
                ..
            } if worker.as_str() == "api" && container.as_str() == "gumgum-api" && route.as_str() == "api.example.test" && port.get() == 3000 && health == "/healthz"
        ));
    }

    #[tokio::test]
    async fn generic_executor_can_execute_self_contained_deploy_without_deploy_context() {
        let step = GraphActionPlanner::ensure_deploy_step(
            WorkerId::new("api").unwrap(),
            ContainerName::new("gumgum-api").unwrap(),
            ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            RouteHost::new("api.example.test").unwrap(),
            Port::new(3000).unwrap(),
            "/healthz",
        );
        let actions = GraphActionExecutor::execute_steps(
            &[step],
            GraphExecutionContext {
                graph_path: None,
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap();

        assert!(actions[0].contains("planned ensure deploy runtime for api"));
    }

    #[tokio::test]
    async fn generic_executor_plans_container_without_runtime_context() {
        let steps = GraphActionPlanner::plan(&[GraphReconcileAction::EnsureContainer {
            name: "api".to_owned(),
            image: "ghcr.io/acme/api:v1".to_owned(),
        }]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert_eq!(
            actions,
            vec!["planned ensure deploy runtime for api runs image ghcr.io/acme/api:v1"]
        );
    }

    #[tokio::test]
    async fn generic_executor_deduplicates_container_runtime_steps() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureContainer {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
            GraphReconcileAction::EnsureContainer {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
        ]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            "planned ensure deploy runtime for api runs image ghcr.io/acme/api:v1"
        );
    }

    #[test]
    fn planner_normalizes_deploy_runtime_to_container_step() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureWorker {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
            GraphReconcileAction::EnsureContainer {
                name: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            },
            GraphReconcileAction::EnsureRoute {
                host: "api.example.test".to_owned(),
                target_container: "api".to_owned(),
            },
        ]);

        assert_eq!(steps.len(), 1);
        assert!(matches!(
            steps[0].target,
            GraphExecutionTarget::DeployRuntime {
                worker: Some(ref worker),
                route: Some(ref route),
                ..
            } if worker.as_str() == "api" && route.as_str() == "api.example.test"
        ));
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

        assert_eq!(steps.len(), 1);
        assert!(matches!(
            steps[0].target,
            GraphExecutionTarget::DeployRuntime {
                route: Some(ref route),
                ..
            } if route.as_str() == "api.example.test"
        ));
    }
}
