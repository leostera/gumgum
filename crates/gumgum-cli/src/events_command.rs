use crate::{
    EventsArgs,
    operations_command::{OperationsReport, operation_line, summarize_operations},
    print_value, resolve_server,
    server_client::ServerClient,
};
use gumgum_core::{ControlPlaneEventKind, ErrorCode, GumgumError, GumgumEvent, Subsystem};
use std::str::FromStr;

pub(crate) async fn events(args: EventsArgs, json: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let mut report = ServerClient::new(server.host).events(args.limit).await?;
    if let Some(kind) = args.kind.as_deref() {
        let kind = ControlPlaneEventKind::from_str(kind).map_err(|_| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "unknown event kind filter",
            )
            .likely_cause("expected mutation or reconciliation")
            .build()
        })?;
        report.events.retain(|event| event.kind == kind);
    }
    if args.grouped {
        let report = OperationsReport {
            ok: true,
            operations: summarize_operations(&report.events),
        };
        if json {
            print_value(true, &report);
        } else if report.operations.is_empty() {
            println!("no operations recorded");
        } else {
            for operation in report.operations {
                println!("{}", operation_line(&operation));
                for target in operation.targets.iter().take(5) {
                    println!("  - {target}");
                }
            }
        }
    } else {
        let typed_events = typed_events_for_report(&report);
        if json {
            for event in typed_events {
                println!(
                    "{}",
                    serde_json::to_string(&event).expect("serialize gumgum event")
                );
            }
        } else if typed_events.is_empty() {
            println!("no control-plane events recorded");
        } else {
            for event in typed_events {
                println!("{}", event_line(&event));
            }
        }
    }
    Ok(())
}

fn typed_events_for_report(report: &gumgum_api::EventsReport) -> Vec<GumgumEvent> {
    if report.typed_events.is_empty() {
        report.events.iter().cloned().map(Into::into).collect()
    } else {
        report.typed_events.clone()
    }
}

fn event_line(event: &GumgumEvent) -> String {
    match event {
        GumgumEvent::DesiredStateMutated {
            id,
            operation_id,
            target,
            message,
            at,
            ..
        } => stored_event_line(
            *id,
            operation_id.as_deref(),
            at.as_deref(),
            "mutation",
            "executed",
            target,
            message,
        ),
        GumgumEvent::ReconcileStepPlanned {
            id,
            operation_id,
            target,
            message,
            at,
            ..
        } => stored_event_line(
            *id,
            operation_id.as_deref(),
            at.as_deref(),
            "reconciliation",
            "planned",
            target,
            message,
        ),
        GumgumEvent::ReconcileStepExecuted {
            id,
            operation_id,
            target,
            message,
            at,
            ..
        } => stored_event_line(
            *id,
            operation_id.as_deref(),
            at.as_deref(),
            "reconciliation",
            "executed",
            target,
            message,
        ),
        GumgumEvent::ReconcileStepFailed {
            id,
            operation_id,
            target,
            message,
            at,
            ..
        } => stored_event_line(
            *id,
            operation_id.as_deref(),
            at.as_deref(),
            "reconciliation",
            "failed",
            target,
            message,
        ),
        GumgumEvent::DeploymentPlanned {
            worker,
            environment,
            image,
            route,
        } => format!(
            "deploy planned {}{} image={}{}",
            worker,
            env_suffix(environment),
            image,
            route
                .as_deref()
                .map(|route| format!(" route={route}"))
                .unwrap_or_default()
        ),
        GumgumEvent::DeploymentStarted {
            worker,
            environment,
            image,
        } => {
            format!(
                "deploy started {}{} image={}",
                worker,
                env_suffix(environment),
                image
            )
        }
        GumgumEvent::DeploymentSucceeded {
            worker,
            environment,
            revision,
            route,
        } => format!(
            "deploy succeeded {}{}{}{}",
            worker,
            env_suffix(environment),
            revision
                .as_deref()
                .map(|revision| format!(" revision={revision}"))
                .unwrap_or_default(),
            route
                .as_deref()
                .map(|route| format!(" route={route}"))
                .unwrap_or_default()
        ),
        GumgumEvent::DeploymentFailed {
            worker,
            environment,
            error,
        } => {
            format!(
                "deploy failed {}{} - {}",
                worker,
                env_suffix(environment),
                error
            )
        }
    }
}

fn env_suffix(environment: &Option<String>) -> String {
    environment
        .as_deref()
        .map(|env| format!("@{env}"))
        .unwrap_or_default()
}

fn stored_event_line(
    id: Option<gumgum_core::ReconcileEventId>,
    operation_id: Option<&str>,
    at: Option<&str>,
    kind: &str,
    status: &str,
    target: &str,
    message: &str,
) -> String {
    let id = id
        .map(|id| format!("#{}", id.get()))
        .unwrap_or_else(|| "#?".to_owned());
    let operation = operation_id
        .map(|operation_id| format!(" op={operation_id}"))
        .unwrap_or_default();
    format!(
        "{}{} {} {} {} {} - {}",
        id,
        operation,
        at.unwrap_or("unknown-time"),
        kind,
        status,
        target,
        message
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{ControlPlaneEventKind, ReconcileEventId, ReconcileEventStatus};

    #[test]
    fn event_line_includes_operation_id_when_present() {
        let event = gumgum_core::ReconcileEvent {
            id: ReconcileEventId::new(7),
            kind: ControlPlaneEventKind::Reconciliation,
            status: ReconcileEventStatus::Executed,
            operation_id: Some("reconcile-123".to_owned()),
            target: "deploy/api".to_owned(),
            action: "ensure_deploy".to_owned(),
            message: "health verified".to_owned(),
            created_at: "2026-05-21 12:00:00".to_owned(),
        };

        assert_eq!(
            event_line(&event.into()),
            "#7 op=reconcile-123 2026-05-21 12:00:00 reconciliation executed deploy/api - health verified"
        );
    }

    #[test]
    fn typed_events_fall_back_to_stored_events_for_old_daemons() {
        let report = gumgum_api::EventsReport {
            ok: true,
            events: vec![gumgum_core::ReconcileEvent {
                id: ReconcileEventId::new(8),
                kind: ControlPlaneEventKind::Mutation,
                status: ReconcileEventStatus::Executed,
                operation_id: None,
                target: "object/db/app".to_owned(),
                action: "object.upsert".to_owned(),
                message: "object materialized".to_owned(),
                created_at: "2026-05-21 12:01:00".to_owned(),
            }],
            typed_events: Vec::new(),
            message: "1 event".to_owned(),
        };

        assert!(matches!(
            typed_events_for_report(&report).first(),
            Some(GumgumEvent::DesiredStateMutated { .. })
        ));
    }
}
