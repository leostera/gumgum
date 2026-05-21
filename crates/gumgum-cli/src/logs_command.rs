use crate::{LogsArgs, print_value, resolve_server};
use gumgum_core::{ErrorCode, GumgumError, Subsystem, load_worker_path, sanitize_name};
use std::{path::Path, time::Duration};

use crate::server_client::ServerClient;

pub(crate) async fn logs(args: LogsArgs, quiet: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?.host;
    let container = logs_container(&args.path)?;
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

fn logs_container(target: &Path) -> gumgum_core::Result<String> {
    if target.exists() {
        let manifest_path = if target.is_dir() {
            target.join("gumgum.toml")
        } else {
            target.to_path_buf()
        };
        let manifest = load_worker_path(&manifest_path)?;
        return Ok(format!("gumgum-{}", sanitize_name(&manifest.worker.name)));
    }
    let target = target.to_string_lossy();
    Ok(format!("gumgum-{}", sanitize_name(&target)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn logs_container_accepts_worker_name_for_workspace_usage() {
        assert_eq!(logs_container(Path::new("api")).unwrap(), "gumgum-api");
    }

    #[test]
    fn logs_container_accepts_worker_directory() {
        let dir = std::env::temp_dir().join(format!(
            "gumgum-logs-manifest-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("gumgum.toml"),
            "[project]\nnamespace = \"visit-counter\"\n\n[worker]\nname = \"api\"\nbuild_context = \".\"\nport = 3000\nhealth = \"/healthz\"\n",
        )
        .unwrap();

        assert_eq!(logs_container(&dir).unwrap(), "gumgum-api");
        let _ = fs::remove_dir_all(dir);
    }
}
