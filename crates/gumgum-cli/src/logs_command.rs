use crate::{LogsArgs, print_value, resolve_server};
use gumgum_core::{ErrorCode, GumgumError, Subsystem, load_worker_path, sanitize_name};
use std::time::Duration;

use crate::server_client::ServerClient;

pub(crate) async fn logs(args: LogsArgs, quiet: bool) -> gumgum_core::Result<()> {
    let manifest = load_worker_path(&args.path)?;
    let server = resolve_server(args.host)?.host;
    let container = format!("gumgum-{}", sanitize_name(&manifest.worker.name));
    if args.follow && quiet {
        return Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::InvalidArgs,
            "gumgum logs -f does not support --json yet",
        )
        .next_command("gumgum logs --json")
        .build());
    }
    if args.follow {
        let mut seen = String::new();
        loop {
            let report = ServerClient::new(&server)
                .logs(&container, args.tail)
                .await?;
            if let Some(delta) = report.logs.strip_prefix(&seen) {
                print!("{delta}");
            } else {
                print!("{}", report.logs);
            }
            seen = report.logs;
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
        return Ok(());
    }
    let report = ServerClient::new(&server)
        .logs(&container, args.tail)
        .await?;
    if quiet {
        print_value(true, &report);
    } else {
        print!("{}", report.logs);
    }
    Ok(())
}
