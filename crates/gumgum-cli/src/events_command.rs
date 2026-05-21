use crate::{EventsArgs, print_value, resolve_server, server_client::ServerClient};

pub(crate) async fn events(args: EventsArgs, json: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host).events(args.limit).await?;
    if json {
        print_value(true, &report);
    } else if report.events.is_empty() {
        println!("no reconciliation events recorded");
    } else {
        for event in report.events {
            println!(
                "#{} {} {} {} - {}",
                event.id.get(),
                event.created_at,
                event.status,
                event.target,
                event.message
            );
        }
    }
    Ok(())
}
