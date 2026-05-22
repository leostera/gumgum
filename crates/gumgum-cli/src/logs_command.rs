use crate::{LogsArgs, print_value, resolve_server};
use gumgum_api::{LogsReport, ServerRecord};
use gumgum_core::{
    DeploymentDescriptor, ErrorCode, GumgumError, ManifestKind, Subsystem, load_worker_path,
    load_workspace_path, sanitize_name, validate_path,
};
use serde::Serialize;
use std::{path::Path, time::Duration};

use crate::server_client::ServerClient;

#[derive(Debug, Serialize)]
struct WorkspaceLogsReport {
    ok: bool,
    workers: Vec<LogsReport>,
}

pub(crate) async fn logs(args: LogsArgs, quiet: bool) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let targets = logs_targets(&args.path, &server)?;
    let server = server.host;
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
        let mut seen = targets
            .iter()
            .map(|target| (target.container.clone(), String::new()))
            .collect::<std::collections::BTreeMap<_, _>>();
        loop {
            for target in &targets {
                let report = ServerClient::new(&server)
                    .logs(&target.container, args.tail)
                    .await?;
                let previous = seen.entry(target.container.clone()).or_default();
                let delta = report
                    .logs
                    .strip_prefix(previous.as_str())
                    .unwrap_or(&report.logs);
                if targets.len() == 1 {
                    print!("{delta}");
                } else {
                    print_prefixed_log_delta(&target.worker, delta);
                }
                *previous = report.logs;
            }
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
        return Ok(());
    }

    let mut reports = Vec::new();
    for target in targets {
        reports.push(
            ServerClient::new(&server)
                .logs(&target.container, args.tail)
                .await?,
        );
    }
    if quiet {
        if reports.len() == 1 {
            print_value(true, &reports[0]);
        } else {
            print_value(
                true,
                &WorkspaceLogsReport {
                    ok: true,
                    workers: reports,
                },
            );
        }
    } else if reports.len() == 1 {
        print!("{}", reports[0].logs);
    } else {
        for report in reports {
            println!("==> {} <==", report.container);
            print!("{}", report.logs);
            if !report.logs.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct LogTarget {
    worker: String,
    container: String,
}

fn logs_targets(target: &Path, server: &ServerRecord) -> gumgum_core::Result<Vec<LogTarget>> {
    if target.exists() {
        let manifest_path = if target.is_dir() {
            target.join("gumgum.toml")
        } else {
            target.to_path_buf()
        };
        return match validate_path(&manifest_path)?.manifest_kind {
            ManifestKind::Worker => Ok(vec![logs_target(&manifest_path, server)?]),
            ManifestKind::Workspace => {
                let workspace = load_workspace_path(&manifest_path)?;
                let root = manifest_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                workspace
                    .workspace
                    .members
                    .iter()
                    .map(|member| {
                        let member_path = root.join(member).join("gumgum.toml");
                        logs_target(&member_path, server)
                    })
                    .collect()
            }
        };
    }
    let target = target.to_string_lossy();
    Ok(vec![LogTarget {
        worker: sanitize_name(&target),
        container: format!("gumgum-{}", sanitize_name(&target)),
    }])
}

fn logs_target(target: &Path, server: &ServerRecord) -> gumgum_core::Result<LogTarget> {
    let manifest_path = if target.is_dir() {
        target.join("gumgum.toml")
    } else {
        target.to_path_buf()
    };
    let manifest = load_worker_path(&manifest_path)?;
    Ok(LogTarget {
        worker: manifest.worker.name.clone(),
        container: DeploymentDescriptor::from_manifest(
            &manifest_path,
            &manifest,
            Some(server),
            false,
        )
        .container,
    })
}

fn print_prefixed_log_delta(worker: &str, delta: &str) {
    for line in delta.split_inclusive('\n') {
        if line.is_empty() {
            continue;
        }
        print!("{}: {}", worker, line);
        if !line.ends_with('\n') {
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn logs_targets_accept_worker_name_for_workspace_usage() {
        assert_eq!(
            logs_targets(Path::new("api"), &server_record()).unwrap()[0].container,
            "gumgum-api"
        );
    }

    #[test]
    fn logs_targets_accept_worker_directory() {
        let dir = temp_dir("worker");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("gumgum.toml"),
            "[project]\nnamespace = \"visit-counter\"\n\n[worker]\nname = \"api\"\nbuild_context = \".\"\nport = 3000\nhealth = \"/healthz\"\n",
        )
        .unwrap();

        assert_eq!(
            logs_targets(&dir, &server_record()).unwrap()[0].container,
            "gumgum-dev-leostera-visit-counter-api"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn logs_targets_expand_workspace_members() {
        let dir = temp_dir("workspace");
        fs::create_dir_all(dir.join("api")).unwrap();
        fs::create_dir_all(dir.join("worker")).unwrap();
        fs::write(
            dir.join("gumgum.toml"),
            "[project]\nname = \"visit-counter\"\ndomain = \"visitcounter.dev\"\n\n[workspace]\nmembers = [\"api\", \"worker\"]\n",
        )
        .unwrap();
        for worker in ["api", "worker"] {
            fs::write(
                dir.join(worker).join("gumgum.toml"),
                format!(
                    "[project]\nnamespace = \"visit-counter\"\n\n[worker]\nname = \"{worker}\"\nbuild_context = \".\"\nport = 3000\nhealth = \"/healthz\"\n"
                ),
            )
            .unwrap();
        }

        let targets = logs_targets(&dir.join("gumgum.toml"), &server_record()).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0].container,
            "gumgum-dev-leostera-visit-counter-api"
        );
        assert_eq!(
            targets[1].container,
            "gumgum-dev-leostera-visit-counter-worker"
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gumgum-logs-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn server_record() -> ServerRecord {
        ServerRecord {
            name: "starbase2".to_owned(),
            host: "192.168.0.3".to_owned(),
            root_domain: "leostera.dev".to_owned(),
            test_domain: "leostera.test".to_owned(),
            health_url: "http://starbase2:7777/healthz".to_owned(),
        }
    }
}
