use gumgum_core::{ControlPlaneEventKind, ReconcileEvent, ReconcileEventStatus};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub(crate) struct OperationsReport {
    pub(crate) ok: bool,
    pub(crate) operations: Vec<OperationSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OperationSummary {
    pub(crate) operation_id: String,
    pub(crate) kind: String,
    pub(crate) latest_status: ReconcileEventStatus,
    pub(crate) latest_at: String,
    pub(crate) event_count: usize,
    pub(crate) failed: usize,
    pub(crate) executed: usize,
    pub(crate) planned: usize,
    pub(crate) targets: Vec<String>,
}

pub(crate) fn summarize_operations(events: &[ReconcileEvent]) -> Vec<OperationSummary> {
    let mut groups: BTreeMap<String, Vec<&ReconcileEvent>> = BTreeMap::new();
    for event in events {
        let operation_id = event
            .operation_id
            .clone()
            .unwrap_or_else(|| format!("event-{}", event.id.get()));
        groups.entry(operation_id).or_default().push(event);
    }

    let mut summaries = groups
        .into_iter()
        .map(|(operation_id, mut events)| {
            events.sort_by_key(|event| event.id.get());
            let latest = events.last().expect("operation group is not empty");
            let failed = events
                .iter()
                .filter(|event| event.status == ReconcileEventStatus::Failed)
                .count();
            let executed = events
                .iter()
                .filter(|event| event.status == ReconcileEventStatus::Executed)
                .count();
            let planned = events
                .iter()
                .filter(|event| event.status == ReconcileEventStatus::Planned)
                .count();
            let mut targets = events
                .iter()
                .map(|event| format!("{} {} - {}", event.status, event.target, event.message))
                .collect::<Vec<_>>();
            targets.dedup();
            OperationSummary {
                operation_id,
                kind: operation_kind(&events),
                latest_status: latest.status,
                latest_at: latest.created_at.clone(),
                event_count: events.len(),
                failed,
                executed,
                planned,
                targets,
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| right.latest_at.cmp(&left.latest_at));
    summaries
}

fn operation_kind(events: &[&ReconcileEvent]) -> String {
    if events
        .iter()
        .any(|event| event.kind == ControlPlaneEventKind::Reconciliation)
    {
        "reconciliation".to_owned()
    } else {
        "mutation".to_owned()
    }
}

pub(crate) fn operation_line(operation: &OperationSummary) -> String {
    format!(
        "{} {} {} events={} planned={} executed={} failed={} latest={}",
        operation.operation_id,
        operation.kind,
        operation.latest_status,
        operation.event_count,
        operation.planned,
        operation.executed,
        operation.failed,
        operation.latest_at
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{ControlPlaneEventKind, ReconcileEventId};

    fn event(id: i64, op: &str, status: ReconcileEventStatus) -> ReconcileEvent {
        ReconcileEvent {
            id: ReconcileEventId::new(id),
            kind: ControlPlaneEventKind::Reconciliation,
            status,
            operation_id: Some(op.to_owned()),
            target: "deploy/api".to_owned(),
            action: "ensure_deploy".to_owned(),
            message: status.to_string(),
            created_at: format!("2026-05-21 12:00:0{id}"),
        }
    }

    #[test]
    fn summarizes_events_by_operation_id() {
        let summaries = summarize_operations(&[
            event(1, "reconcile-1", ReconcileEventStatus::Planned),
            event(2, "reconcile-1", ReconcileEventStatus::Executed),
        ]);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].operation_id, "reconcile-1");
        assert_eq!(summaries[0].planned, 1);
        assert_eq!(summaries[0].executed, 1);
        assert_eq!(summaries[0].latest_status, ReconcileEventStatus::Executed);
    }
}
