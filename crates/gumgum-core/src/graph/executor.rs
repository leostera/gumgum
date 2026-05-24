use crate::{
    Capability, ContainerName, DeployRequest, DesiredGraph, GraphNodeId, GraphReconcileAction,
    GraphReconciler, HealthPath, ImageName, ObjectName, ObjectProviderPlan, Port,
    ProviderCredentials, ProviderName, RouteHost, WorkerId,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "target")]
pub enum GraphExecutionTarget {
    Provider {
        name: ProviderName,
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
        project: Option<String>,
        domain: Option<String>,
        port: Option<Port>,
        health: Option<HealthPath>,
    },
    Gateway {
        host: String,
        target_container: String,
    },
    ObjectProvider {
        capability: Capability,
        name: ObjectName,
        provider: ProviderName,
    },
    ObjectProviderRemoval {
        capability: Capability,
        name: ObjectName,
        provider: ProviderName,
    },
    GraphStore {
        node: String,
    },
    Removal {
        id: GraphNodeId,
        container: Option<ContainerName>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GraphExecutionStep {
    pub action: GraphReconcileAction,
    pub target: GraphExecutionTarget,
    pub description: String,
}

pub type GumgumAction = GraphExecutionStep;

impl GraphExecutionStep {
    pub fn planned_event(&self, operation_id: Option<String>) -> crate::GumgumEvent {
        self.reconcile_event(
            crate::ReconcileEventStatus::Planned,
            operation_id,
            self.description.clone(),
        )
    }

    pub fn executed_event(
        &self,
        operation_id: Option<String>,
        message: impl Into<String>,
    ) -> crate::GumgumEvent {
        self.reconcile_event(
            crate::ReconcileEventStatus::Executed,
            operation_id,
            message.into(),
        )
    }

    pub fn failed_event(
        &self,
        operation_id: Option<String>,
        message: impl Into<String>,
    ) -> crate::GumgumEvent {
        self.reconcile_event(
            crate::ReconcileEventStatus::Failed,
            operation_id,
            message.into(),
        )
    }

    fn reconcile_event(
        &self,
        status: crate::ReconcileEventStatus,
        operation_id: Option<String>,
        message: String,
    ) -> crate::GumgumEvent {
        let id = None;
        let target = self.target.event_target();
        let action = self.action.event_action();
        let at = None;
        match status {
            crate::ReconcileEventStatus::Planned => crate::GumgumEvent::ReconcileStepPlanned {
                id,
                operation_id,
                target,
                action,
                message,
                at,
            },
            crate::ReconcileEventStatus::Executed => crate::GumgumEvent::ReconcileStepExecuted {
                id,
                operation_id,
                target,
                action,
                message,
                at,
            },
            crate::ReconcileEventStatus::Failed => crate::GumgumEvent::ReconcileStepFailed {
                id,
                operation_id,
                target,
                action,
                message,
                at,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GraphReconciliationPlan {
    pub actions: Vec<GraphReconcileAction>,
    pub steps: Vec<GraphExecutionStep>,
}

/// The graph of executable actions derived by comparing current state to desired state.
///
/// Keep the control-plane vocabulary explicit:
///
/// ```text
/// CurrentGraph + DesiredGraph = ActionGraph
/// ```
pub type ActionGraph = GraphReconciliationPlan;

/// Snapshot of the currently-known graph state used as the left-hand side of planning.
pub type CurrentGraph = DesiredGraph;

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
                        project: None,
                        domain: None,
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
        current_graph: &CurrentGraph,
        desired_graph: &DesiredGraph,
    ) -> ActionGraph {
        let actions = GraphReconciler::reconcile(current_graph, desired_graph);
        let steps = Self::plan(&actions);
        ActionGraph { actions, steps }
    }

    pub fn ensure_provider_step(name: ProviderName, capability: Capability) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureProvider { name, capability })
    }

    pub fn ensure_object_step(
        capability: Capability,
        name: ObjectName,
        provider: ProviderName,
    ) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureObject {
            capability,
            name,
            provider,
        })
    }

    pub fn ensure_container_step(name: ContainerName, image: ImageName) -> GraphExecutionStep {
        plan_action(&GraphReconcileAction::EnsureContainer { name, image })
    }

    pub fn ensure_deploy_step(
        worker: WorkerId,
        container: ContainerName,
        image: ImageName,
        route: Option<RouteHost>,
        port: Port,
        health: HealthPath,
    ) -> GraphExecutionStep {
        GraphExecutionStep {
            action: GraphReconcileAction::EnsureContainer {
                name: container.clone(),
                image: image.clone(),
            },
            target: GraphExecutionTarget::DeployRuntime {
                worker: Some(worker.clone()),
                container,
                image: image.clone(),
                route,
                project: None,
                domain: None,
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
    pub graph_path: Option<PathBuf>,
    pub event_sender: Option<mpsc::UnboundedSender<crate::GumgumEvent>>,
    #[cfg(test)]
    pub fail_next_step: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct GraphExecutionReport {
    pub actions: crate::CoreActions,
    pub typed_events: Vec<crate::GumgumEvent>,
}

pub struct GraphActionExecutor;

impl GraphActionExecutor {
    pub async fn execute_steps(
        steps: &[GraphExecutionStep],
        context: GraphExecutionContext,
    ) -> crate::Result<crate::CoreActions> {
        Ok(Self::execute_steps_report(steps, context).await?.actions)
    }

    pub async fn execute_steps_report(
        steps: &[GraphExecutionStep],
        context: GraphExecutionContext,
    ) -> crate::Result<GraphExecutionReport> {
        GraphExecutionSession::new(context).execute(steps).await
    }

    pub async fn execute_provider_steps(
        steps: &[GraphExecutionStep],
    ) -> crate::Result<crate::CoreActions> {
        Self::execute_steps(steps, GraphExecutionContext::default()).await
    }

    pub async fn execute_provider(
        name: &str,
        capability: Capability,
    ) -> crate::Result<crate::CoreActions> {
        match (name, capability) {
            ("secrets.platform", Capability::Secret) => {
                crate::providers::vaultwarden::ensure().await
            }
            _ => Ok(vec![crate::CoreAction::ProviderConfigured {
                capability,
                provider: name.to_owned(),
            }]),
        }
    }

    pub async fn execute_object_provider_steps(
        steps: &[GraphExecutionStep],
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<crate::CoreActions> {
        Self::execute_steps(
            steps,
            GraphExecutionContext {
                object_plan: Some(plan.clone()),
                provider_credentials: credentials,
                graph_path: None,
                event_sender: None,
                #[cfg(test)]
                fail_next_step: false,
            },
        )
        .await
    }

    pub async fn execute_object_plan(
        plan: &ObjectProviderPlan,
        credentials: Option<ProviderCredentials>,
    ) -> crate::Result<crate::CoreActions> {
        crate::providers::ProviderReconciler::ensure_with_credentials(plan, credentials).await
    }
}

struct GraphExecutionSession {
    context: GraphExecutionContext,
    container_runtime_seen: bool,
    operation_id: Option<String>,
}

impl GraphExecutionSession {
    fn new(context: GraphExecutionContext) -> Self {
        let operation_id = context
            .graph_path
            .as_ref()
            .map(|_| crate::new_operation_id("reconcile"));
        Self {
            context,
            container_runtime_seen: false,
            operation_id,
        }
    }

    async fn execute(
        mut self,
        steps: &[GraphExecutionStep],
    ) -> crate::Result<GraphExecutionReport> {
        let mut report = GraphExecutionReport::default();
        for step in steps {
            self.record(
                crate::ReconcileEventStatus::Planned,
                step,
                step.description.clone(),
            )?;
            let planned_event = step.planned_event(self.operation_id.clone());
            self.emit_event(planned_event.clone());
            report.typed_events.push(planned_event);
            match self.execute_step(step).await {
                Ok(step_actions) => {
                    let message = serde_json::to_string(&step_actions).unwrap_or_default();
                    self.record(crate::ReconcileEventStatus::Executed, step, message.clone())?;
                    let executed_event = step.executed_event(self.operation_id.clone(), message);
                    self.emit_event(executed_event.clone());
                    report.typed_events.push(executed_event);
                    report.actions.extend(step_actions);
                }
                Err(error) => {
                    let message = error.to_report().message.clone();
                    self.record(crate::ReconcileEventStatus::Failed, step, message.clone())?;
                    let failed_event = step.failed_event(self.operation_id.clone(), message);
                    self.emit_event(failed_event.clone());
                    report.typed_events.push(failed_event);
                    return Err(error);
                }
            }
        }
        Ok(report)
    }

    fn emit_event(&self, event: crate::GumgumEvent) {
        if let Some(sender) = &self.context.event_sender {
            let _ = sender.send(event);
        }
    }

    async fn execute_step(
        &mut self,
        step: &GraphExecutionStep,
    ) -> crate::Result<crate::CoreActions> {
        #[cfg(test)]
        if self.context.fail_next_step {
            self.context.fail_next_step = false;
            return Err(crate::GumgumError::structured(
                crate::Subsystem::Setup,
                crate::ErrorCode::InvalidArgs,
                "injected graph execution failure",
            )
            .build());
        }

        match &step.target {
            GraphExecutionTarget::Provider { name, capability } => {
                GraphActionExecutor::execute_provider(name.as_str(), *capability).await
            }
            GraphExecutionTarget::ObjectProvider { .. } => {
                if let Some(plan) = &self.context.object_plan {
                    GraphActionExecutor::execute_object_plan(
                        plan,
                        self.context.provider_credentials.clone(),
                    )
                    .await
                } else {
                    Ok(vec![crate::CoreAction::Planned {
                        target: step.target.event_target(),
                        action: step.action.event_action(),
                    }])
                }
            }
            GraphExecutionTarget::ObjectProviderRemoval { .. } => {
                if let Some(plan) = &self.context.object_plan {
                    crate::providers::ProviderReconciler::delete_with_credentials(
                        plan,
                        self.context.provider_credentials.clone(),
                    )
                    .await
                } else {
                    Ok(vec![crate::CoreAction::Planned {
                        target: step.target.event_target(),
                        action: step.action.event_action(),
                    }])
                }
            }
            GraphExecutionTarget::ContainerRuntime { .. }
            | GraphExecutionTarget::DeployRuntime { .. } => self.execute_deploy_step(step).await,
            GraphExecutionTarget::WorkerRuntime { .. }
            | GraphExecutionTarget::Gateway { .. }
            | GraphExecutionTarget::GraphStore { .. } => Ok(vec![crate::CoreAction::Planned {
                target: step.target.event_target(),
                action: step.action.event_action(),
            }]),
            GraphExecutionTarget::Removal { .. } => self.execute_removal_step(step).await,
        }
    }

    async fn execute_deploy_step(
        &mut self,
        step: &GraphExecutionStep,
    ) -> crate::Result<crate::CoreActions> {
        if self.container_runtime_seen {
            return Ok(vec![crate::CoreAction::Planned {
                target: step.target.event_target(),
                action: step.action.event_action(),
            }]);
        }
        self.container_runtime_seen = true;
        if let Some(graph_path) = &self.context.graph_path {
            let Some(request) = step.target.deploy_request() else {
                return Ok(vec![crate::CoreAction::Planned {
                    target: step.target.event_target(),
                    action: step.action.event_action(),
                }]);
            };
            let (changed, mut deploy_actions) = crate::ContainerReconciler::new(graph_path.clone())
                .reconcile(&request)
                .await?;
            if !changed && deploy_actions.is_empty() {
                deploy_actions.push(crate::CoreAction::DeploymentContainerMatches {
                    container: request.container.clone(),
                });
            }
            Ok(deploy_actions)
        } else {
            Ok(vec![crate::CoreAction::Planned {
                target: step.target.event_target(),
                action: step.action.event_action(),
            }])
        }
    }

    async fn execute_removal_step(
        &self,
        step: &GraphExecutionStep,
    ) -> crate::Result<crate::CoreActions> {
        let GraphExecutionTarget::Removal { id, container } = &step.target else {
            return Ok(vec![crate::CoreAction::Planned {
                target: step.target.event_target(),
                action: step.action.event_action(),
            }]);
        };
        let docker = crate::DockerEngine::local()?;
        let mut containers = Vec::new();
        if let Some(worker) = id.as_str().strip_prefix("deployment/") {
            containers.extend(
                docker
                    .list_container_names_by_label(&[
                        "gumgum.managed=deployment".to_owned(),
                        format!("gumgum.worker={worker}"),
                    ])
                    .await?,
            );
        }
        if let Some(container) = container {
            containers.push(container.to_string());
        }
        containers.sort();
        containers.dedup();
        if containers.is_empty() {
            return Ok(vec![crate::CoreAction::Planned {
                target: step.target.event_target(),
                action: step.action.event_action(),
            }]);
        }
        let mut actions = Vec::new();
        for container in containers {
            docker.remove_container_force(&container).await?;
            actions.push(crate::CoreAction::DeploymentContainerRemoved { container });
        }
        Ok(actions)
    }

    fn record(
        &self,
        status: crate::ReconcileEventStatus,
        step: &GraphExecutionStep,
        message: String,
    ) -> crate::Result<()> {
        let Some(graph_path) = &self.context.graph_path else {
            return Ok(());
        };
        crate::GraphStore::new(graph_path.clone()).record_reconcile_event(
            &crate::NewReconcileEvent {
                kind: crate::ControlPlaneEventKind::Reconciliation,
                status,
                operation_id: self.operation_id.clone(),
                target: step.target.event_target(),
                action: step.action.event_action(),
                message,
            },
        )?;
        Ok(())
    }
}

impl GraphExecutionTarget {
    fn deploy_request(&self) -> Option<DeployRequest> {
        match self {
            GraphExecutionTarget::DeployRuntime {
                worker: Some(worker),
                container,
                image,
                route,
                project,
                domain,
                port: Some(port),
                health: Some(health),
            } => Some(DeployRequest {
                worker: worker.to_string(),
                image: image.to_string(),
                container: container.to_string(),
                route: route.as_ref().map(ToString::to_string),
                project: project.clone(),
                domain: domain.clone(),
                port: port.get(),
                health: health.to_string(),
            }),
            _ => None,
        }
    }

    fn event_target(&self) -> String {
        match self {
            Self::Provider { name, .. } => format!("provider/{name}"),
            Self::WorkerRuntime { worker, .. } => format!("worker/{worker}"),
            Self::ContainerRuntime { container, .. } => format!("container/{container}"),
            Self::DeployRuntime {
                worker: Some(worker),
                ..
            } => format!("deployment/{worker}"),
            Self::DeployRuntime { container, .. } => format!("container/{container}"),
            Self::Gateway { host, .. } => format!("route/{host}"),
            Self::ObjectProvider {
                capability, name, ..
            }
            | Self::ObjectProviderRemoval {
                capability, name, ..
            } => format!("{capability}/{name}"),
            Self::GraphStore { node } => node.clone(),
            Self::Removal { id, .. } => id.to_string(),
        }
    }
}

impl GraphReconcileAction {
    fn event_action(&self) -> String {
        match self {
            Self::EnsureProvider { .. } => "ensure_provider".to_owned(),
            Self::EnsureWorker { .. } => "ensure_worker".to_owned(),
            Self::EnsureContainer { .. } => "ensure_container".to_owned(),
            Self::EnsureDeploy { .. } => "ensure_deploy".to_owned(),
            Self::EnsureRoute { .. } => "ensure_route".to_owned(),
            Self::EnsureBinding { .. } => "ensure_binding".to_owned(),
            Self::EnsureObject { .. } => "ensure_object".to_owned(),
            Self::RemoveObject { .. } => "remove_object".to_owned(),
            Self::RemoveNode { .. } => "remove_node".to_owned(),
            Self::RemoveDeploy { .. } => "remove_deploy".to_owned(),
        }
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
                worker: name.to_string(),
                image: image.to_string(),
            },
            description: format!("ensure worker {name} is represented in desired runtime state"),
        },
        GraphReconcileAction::EnsureContainer { name, image } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::ContainerRuntime {
                container: name.to_string(),
                image: image.to_string(),
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
                route: route.clone(),
                project: None,
                domain: None,
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
                host: host.to_string(),
                target_container: target_container.to_string(),
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
        GraphReconcileAction::RemoveObject {
            capability,
            name,
            provider,
        } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::ObjectProviderRemoval {
                capability: *capability,
                name: name.clone(),
                provider: provider.clone(),
            },
            description: format!("remove {capability} object {name} from {provider}"),
        },
        GraphReconcileAction::RemoveNode { id } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Removal {
                id: id.clone(),
                container: None,
            },
            description: format!("remove graph node {id}"),
        },
        GraphReconcileAction::RemoveDeploy { worker, container } => GraphExecutionStep {
            action: action.clone(),
            target: GraphExecutionTarget::Removal {
                id: GraphNodeId::new(format!("deployment/{worker}"))
                    .unwrap_or_else(|_| GraphNodeId::new("deployment/unknown").unwrap()),
                container: Some(container.clone()),
            },
            description: format!("remove deployment {worker} and container {container}"),
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
                name: ProviderName::new("secrets.platform").unwrap(),
                capability: Capability::Secret,
            },
            crate::DesiredGraphNode::Object {
                capability: Capability::Secret,
                name: ObjectName::new("stripe-api-key").unwrap(),
                provider: ProviderName::new("secrets.platform").unwrap(),
            },
        ]);

        let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);

        assert_eq!(plan.actions.len(), 2);
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.steps.iter().any(|step| matches!(
            step.target,
            GraphExecutionTarget::Provider { ref name, capability: Capability::Secret }
                if name.as_str() == "secrets.platform"
        )));
    }

    #[test]
    fn planner_can_synthesize_idempotent_provider_step() {
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("secrets.platform").unwrap(),
            Capability::Secret,
        );

        assert_eq!(
            step.target,
            GraphExecutionTarget::Provider {
                name: ProviderName::new("secrets.platform").unwrap(),
                capability: Capability::Secret,
            }
        );
        assert_eq!(
            step.description,
            "ensure provider secrets.platform exists and is running"
        );
    }

    #[test]
    fn execution_steps_project_to_typed_events() {
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("manual.main").unwrap(),
            Capability::Manual,
        );

        let event = step.planned_event(Some("reconcile-123".to_owned()));

        assert!(matches!(
            event,
            crate::GumgumEvent::ReconcileStepPlanned {
                operation_id: Some(ref operation_id),
                ref target,
                ref action,
                ..
            } if operation_id == "reconcile-123"
                && target == "provider/manual.main"
                && action == "ensure_provider"
        ));
    }

    #[tokio::test]
    async fn executor_routes_vaultwarden_provider_target_to_backend() {
        let actions = GraphActionExecutor::execute_provider("manual.main", Capability::Manual)
            .await
            .unwrap();

        assert!(matches!(actions.as_slice(), [crate::CoreAction::ProviderConfigured { provider, .. }] if provider == "manual.main"));
    }

    #[tokio::test]
    async fn executor_records_failed_events_when_a_step_errors() {
        let graph_path = std::env::temp_dir().join(format!(
            "gumgum-executor-failed-events-{}.sqlite",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("manual.main").unwrap(),
            Capability::Manual,
        );

        let error = GraphActionExecutor::execute_steps(
            &[step],
            GraphExecutionContext {
                graph_path: Some(graph_path.clone()),
                fail_next_step: true,
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_report().message,
            "injected graph execution failure"
        );
        let events = crate::GraphStore::new(graph_path.clone())
            .list_reconcile_events(10)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].status, crate::ReconcileEventStatus::Failed);
        assert_eq!(events[0].target, "provider/manual.main");
        assert_eq!(events[0].action, "ensure_provider");
        assert_eq!(events[0].message, "injected graph execution failure");
        assert!(events[0].operation_id.is_some());
        assert_eq!(events[0].operation_id, events[1].operation_id);
        assert_eq!(events[1].status, crate::ReconcileEventStatus::Planned);
        let _ = std::fs::remove_file(graph_path);
    }

    #[tokio::test]
    async fn executor_returns_typed_events_as_it_executes_steps() {
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("manual.main").unwrap(),
            Capability::Manual,
        );

        let report =
            GraphActionExecutor::execute_steps_report(&[step], GraphExecutionContext::default())
                .await
                .unwrap();

        assert!(matches!(
            report.actions.as_slice(),
            [crate::CoreAction::ProviderConfigured { provider, .. }] if provider == "manual.main"
        ));
        assert_eq!(report.typed_events.len(), 2);
        assert!(matches!(
            report.typed_events[0],
            crate::GumgumEvent::ReconcileStepPlanned { ref target, ref action, .. }
                if target == "provider/manual.main" && action == "ensure_provider"
        ));
        assert!(matches!(
            report.typed_events[1],
            crate::GumgumEvent::ReconcileStepExecuted { ref target, ref message, .. }
                if target == "provider/manual.main"
                    && message.contains("provider_configured")
        ));
    }

    #[tokio::test]
    async fn executor_sends_typed_events_while_it_executes_steps() {
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("manual.main").unwrap(),
            Capability::Manual,
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();

        let report = GraphActionExecutor::execute_steps_report(
            &[step],
            GraphExecutionContext {
                event_sender: Some(sender),
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap();

        let first = receiver.recv().await.unwrap();
        let second = receiver.recv().await.unwrap();
        assert_eq!(report.typed_events, vec![first.clone(), second.clone()]);
        assert!(matches!(
            first,
            crate::GumgumEvent::ReconcileStepPlanned { ref target, .. }
                if target == "provider/manual.main"
        ));
        assert!(matches!(
            second,
            crate::GumgumEvent::ReconcileStepExecuted { ref target, .. }
                if target == "provider/manual.main"
        ));
    }

    #[tokio::test]
    async fn executor_records_planned_and_executed_events_when_graph_path_is_present() {
        let graph_path = std::env::temp_dir().join(format!(
            "gumgum-executor-events-{}.sqlite",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let step = GraphActionPlanner::ensure_provider_step(
            ProviderName::new("manual.main").unwrap(),
            Capability::Manual,
        );

        let actions = GraphActionExecutor::execute_steps(
            &[step],
            GraphExecutionContext {
                graph_path: Some(graph_path.clone()),
                ..GraphExecutionContext::default()
            },
        )
        .await
        .unwrap();

        assert!(matches!(actions.as_slice(), [crate::CoreAction::ProviderConfigured { provider, .. }] if provider == "manual.main"));
        let events = crate::GraphStore::new(graph_path.clone())
            .list_reconcile_events(10)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].status, crate::ReconcileEventStatus::Executed);
        assert_eq!(events[0].target, "provider/manual.main");
        assert_eq!(events[0].action, "ensure_provider");
        assert!(events[0].operation_id.is_some());
        assert_eq!(events[0].operation_id, events[1].operation_id);
        assert_eq!(events[1].status, crate::ReconcileEventStatus::Planned);
        let _ = std::fs::remove_file(graph_path);
    }

    #[test]
    fn planner_routes_provider_actions_to_provider_executor_target() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureProvider {
                name: ProviderName::new("secrets.platform").unwrap(),
                capability: Capability::Secret,
            },
            GraphReconcileAction::EnsureObject {
                capability: Capability::Blob,
                name: ObjectName::new("uploads").unwrap(),
                provider: ProviderName::new("minio.main").unwrap(),
            },
        ]);

        assert_eq!(
            steps[0].target,
            GraphExecutionTarget::Provider {
                name: ProviderName::new("secrets.platform").unwrap(),
                capability: Capability::Secret,
            }
        );
        assert_eq!(
            steps[1].target,
            GraphExecutionTarget::ObjectProvider {
                capability: Capability::Blob,
                name: ObjectName::new("uploads").unwrap(),
                provider: ProviderName::new("minio.main").unwrap(),
            }
        );
    }

    #[test]
    fn planner_can_synthesize_idempotent_object_step() {
        let step = GraphActionPlanner::ensure_object_step(
            Capability::Blob,
            ObjectName::new("uploads").unwrap(),
            ProviderName::new("minio.main").unwrap(),
        );

        assert_eq!(
            step.target,
            GraphExecutionTarget::ObjectProvider {
                capability: Capability::Blob,
                name: ObjectName::new("uploads").unwrap(),
                provider: ProviderName::new("minio.main").unwrap(),
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
            ObjectName::new("stripe-api-key").unwrap(),
            ProviderName::new("secrets.platform").unwrap(),
        );
        let actions = GraphActionExecutor::execute_object_provider_steps(&[step], &plan, None)
            .await
            .unwrap();

        assert!(actions.iter().any(|action| matches!(
            action,
            crate::CoreAction::ProviderObjectDesiredRemoved { capability: Capability::Secret, .. }
        )));
    }

    #[tokio::test]
    async fn generic_executor_dispatches_provider_and_plans_runtime_targets() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureProvider {
                name: ProviderName::new("manual.main").unwrap(),
                capability: Capability::Manual,
            },
            GraphReconcileAction::EnsureRoute {
                host: RouteHost::new("api.example.test").unwrap(),
                target_container: ContainerName::new("api").unwrap(),
            },
        ]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert!(matches!(
            actions.first(),
            Some(crate::CoreAction::ProviderConfigured { provider, .. }) if provider == "manual.main"
        ));
        assert!(matches!(
            actions.get(1),
            Some(crate::CoreAction::Planned { target, .. }) if target == "route/api.example.test"
        ));
    }

    #[test]
    fn planner_can_synthesize_idempotent_container_step() {
        let step = GraphActionPlanner::ensure_container_step(
            ContainerName::new("api").unwrap(),
            ImageName::new("ghcr.io/acme/api:v1").unwrap(),
        );

        assert_eq!(
            step.target,
            GraphExecutionTarget::ContainerRuntime {
                container: "api".to_owned(),
                image: "ghcr.io/acme/api:v1".to_owned(),
            }
        );
    }

    #[test]
    fn planner_routes_deploy_removal_to_container_cleanup_target() {
        let step = GraphActionPlanner::plan(&[GraphReconcileAction::RemoveDeploy {
            worker: WorkerId::new("api").unwrap(),
            container: ContainerName::new("gumgum-api").unwrap(),
        }])
        .into_iter()
        .next()
        .unwrap();

        assert!(matches!(
            step.target,
            GraphExecutionTarget::Removal {
                ref id,
                container: Some(ref container),
            } if id.as_str() == "deployment/api" && container.as_str() == "gumgum-api"
        ));
        assert_eq!(
            step.description,
            "remove deployment api and container gumgum-api"
        );
    }

    #[test]
    fn planner_can_synthesize_idempotent_deploy_step() {
        let step = GraphActionPlanner::ensure_deploy_step(
            WorkerId::new("api").unwrap(),
            ContainerName::new("gumgum-api").unwrap(),
            ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            Some(RouteHost::new("api.example.test").unwrap()),
            Port::new(3000).unwrap(),
            HealthPath::new("/healthz").unwrap(),
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
            } if worker.as_str() == "api" && container.as_str() == "gumgum-api" && route.as_str() == "api.example.test" && port.get() == 3000 && health.as_str() == "/healthz"
        ));
    }

    #[tokio::test]
    async fn generic_executor_can_execute_self_contained_deploy_without_deploy_context() {
        let step = GraphActionPlanner::ensure_deploy_step(
            WorkerId::new("api").unwrap(),
            ContainerName::new("gumgum-api").unwrap(),
            ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            Some(RouteHost::new("api.example.test").unwrap()),
            Port::new(3000).unwrap(),
            HealthPath::new("/healthz").unwrap(),
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

        assert!(matches!(actions.first(), Some(crate::CoreAction::Planned { target, .. }) if target == "deployment/api"));
    }

    #[tokio::test]
    async fn generic_executor_plans_container_without_runtime_context() {
        let steps = GraphActionPlanner::plan(&[GraphReconcileAction::EnsureContainer {
            name: ContainerName::new("api").unwrap(),
            image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
        }]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert!(matches!(
            actions.as_slice(),
            [crate::CoreAction::Planned { target, .. }] if target == "deployment/api"
        ));
    }

    #[tokio::test]
    async fn generic_executor_deduplicates_container_runtime_steps() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureContainer {
                name: ContainerName::new("api").unwrap(),
                image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            },
            GraphReconcileAction::EnsureContainer {
                name: ContainerName::new("api").unwrap(),
                image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            },
        ]);

        let actions = GraphActionExecutor::execute_steps(&steps, GraphExecutionContext::default())
            .await
            .unwrap();

        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions.first(),
            Some(crate::CoreAction::Planned { target, .. }) if target == "deployment/api"
        ));
    }

    #[test]
    fn planner_normalizes_deploy_runtime_to_container_step() {
        let steps = GraphActionPlanner::plan(&[
            GraphReconcileAction::EnsureWorker {
                name: WorkerId::new("api").unwrap(),
                image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            },
            GraphReconcileAction::EnsureContainer {
                name: ContainerName::new("api").unwrap(),
                image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            },
            GraphReconcileAction::EnsureRoute {
                host: RouteHost::new("api.example.test").unwrap(),
                target_container: ContainerName::new("api").unwrap(),
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
                name: ContainerName::new("api").unwrap(),
                image: ImageName::new("ghcr.io/acme/api:v1").unwrap(),
            },
            GraphReconcileAction::EnsureRoute {
                host: RouteHost::new("api.example.test").unwrap(),
                target_container: ContainerName::new("api").unwrap(),
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
