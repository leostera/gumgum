use crate::{ControlPlaneEventKind, ReconcileEvent, ReconcileEventId, ReconcileEventStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GumgumEvent {
    DesiredStateMutated {
        id: Option<ReconcileEventId>,
        operation_id: Option<String>,
        target: String,
        action: String,
        message: String,
        at: Option<String>,
    },
    ReconcileStepPlanned {
        id: Option<ReconcileEventId>,
        operation_id: Option<String>,
        target: String,
        action: String,
        message: String,
        at: Option<String>,
    },
    ReconcileStepExecuted {
        id: Option<ReconcileEventId>,
        operation_id: Option<String>,
        target: String,
        action: String,
        message: String,
        at: Option<String>,
    },
    ReconcileStepFailed {
        id: Option<ReconcileEventId>,
        operation_id: Option<String>,
        target: String,
        action: String,
        message: String,
        at: Option<String>,
    },
    DeploymentPlanned {
        worker: String,
        environment: Option<String>,
        image: String,
        route: Option<String>,
    },
    DeploymentStarted {
        worker: String,
        environment: Option<String>,
        image: String,
    },
    DeploymentSucceeded {
        worker: String,
        environment: Option<String>,
        revision: Option<String>,
        route: Option<String>,
    },
    DeploymentFailed {
        worker: String,
        environment: Option<String>,
        error: String,
    },
}

impl GumgumEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::DesiredStateMutated { .. } => "desired_state_mutated",
            Self::ReconcileStepPlanned { .. } => "reconcile_step_planned",
            Self::ReconcileStepExecuted { .. } => "reconcile_step_executed",
            Self::ReconcileStepFailed { .. } => "reconcile_step_failed",
            Self::DeploymentPlanned { .. } => "deployment_planned",
            Self::DeploymentStarted { .. } => "deployment_started",
            Self::DeploymentSucceeded { .. } => "deployment_succeeded",
            Self::DeploymentFailed { .. } => "deployment_failed",
        }
    }
}

impl From<ReconcileEvent> for GumgumEvent {
    fn from(event: ReconcileEvent) -> Self {
        let id = Some(event.id);
        let operation_id = event.operation_id;
        let target = event.target;
        let action = event.action;
        let message = event.message;
        let at = Some(event.created_at);
        match (event.kind, event.status) {
            (ControlPlaneEventKind::Mutation, _) => Self::DesiredStateMutated {
                id,
                operation_id,
                target,
                action,
                message,
                at,
            },
            (ControlPlaneEventKind::Reconciliation, ReconcileEventStatus::Planned) => {
                Self::ReconcileStepPlanned {
                    id,
                    operation_id,
                    target,
                    action,
                    message,
                    at,
                }
            }
            (ControlPlaneEventKind::Reconciliation, ReconcileEventStatus::Executed) => {
                Self::ReconcileStepExecuted {
                    id,
                    operation_id,
                    target,
                    action,
                    message,
                    at,
                }
            }
            (ControlPlaneEventKind::Reconciliation, ReconcileEventStatus::Failed) => {
                Self::ReconcileStepFailed {
                    id,
                    operation_id,
                    target,
                    action,
                    message,
                    at,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_reconcile_events_project_to_typed_events() {
        let event = GumgumEvent::from(ReconcileEvent {
            id: ReconcileEventId::new(7),
            kind: ControlPlaneEventKind::Reconciliation,
            status: ReconcileEventStatus::Executed,
            operation_id: Some("op_123".to_owned()),
            target: "deployment/api".to_owned(),
            action: "deploy.apply".to_owned(),
            message: serde_json::json!({"actions":["deployment_applied"]}).to_string(),
            created_at: "2026-05-22T00:00:00Z".to_owned(),
        });

        assert_eq!(event.kind(), "reconcile_step_executed");
        assert!(matches!(event, GumgumEvent::ReconcileStepExecuted { .. }));
    }

    #[test]
    fn typed_events_serialize_as_json_stream_records() {
        let event = GumgumEvent::DeploymentStarted {
            worker: "api".to_owned(),
            environment: Some("preview".to_owned()),
            image: "localhost:5000/api:abc123".to_owned(),
        };

        let json = serde_json::to_string(&event).expect("typed event serializes");
        assert!(json.contains(r#""type":"deployment_started""#));
        assert!(json.contains(r#""environment":"preview""#));
    }
}
