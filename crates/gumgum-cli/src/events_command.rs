use crate::{
    EventsArgs,
    event_presenter::{event_line, print_events_json_lines, typed_events_for_report},
    operations_command::{OperationsReport, operation_line, summarize_operations},
    print_value, resolve_server,
    server_client::ServerClient,
};
use gumgum_core::{ControlPlaneEventKind, ErrorCode, GumgumError, Subsystem};
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
        report.typed_events.retain(|event| match kind {
            ControlPlaneEventKind::Mutation => {
                matches!(event, gumgum_core::GumgumEvent::DesiredStateMutated { .. })
            }
            ControlPlaneEventKind::Reconciliation => {
                !matches!(event, gumgum_core::GumgumEvent::DesiredStateMutated { .. })
            }
        });
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
            print_events_json_lines(typed_events);
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
