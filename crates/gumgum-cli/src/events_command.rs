use crate::{EventsArgs, print_value, resolve_server, server_client::ServerClient};

pub(crate) async fn events(args: EventsArgs, json: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host).events(args.limit).await?;
    if json {
        print_value(true, &report);
    } else if report.events.is_empty() {
        println!("no control-plane events recorded");
    } else {
        for event in report.events {
            println!("{}", event_line(&event));
        }
    }
    Ok(())
}

fn event_line(event: &gumgum_core::ReconcileEvent) -> String {
    let operation = event
        .operation_id
        .as_deref()
        .map(|operation_id| format!(" op={operation_id}"))
        .unwrap_or_default();
    format!(
        "#{}{} {} {} {} {} - {}",
        event.id.get(),
        operation,
        event.created_at,
        event.kind,
        event.status,
        event.target,
        event.message
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
            event_line(&event),
            "#7 op=reconcile-123 2026-05-21 12:00:00 reconciliation executed deploy/api - health verified"
        );
    }
}
