use crate::{EnvArgs, print_value, resolve_server};
use gumgum_api::EnvReport;
use gumgum_core::{
    ErrorCode, GumgumError, ManifestKind, Subsystem, load_worker_path, load_workspace_path,
    sanitize_name, validate_path,
};
use serde::Serialize;
use std::path::Path;

use crate::server_client::ServerClient;

#[derive(Debug, Serialize)]
struct WorkspaceEnvReport {
    ok: bool,
    project: String,
    workers: Vec<EnvReport>,
}

pub(crate) async fn env(args: EnvArgs, json: bool) -> gumgum_core::Result<()> {
    let targets = env_targets(&args.path, args.project.as_deref(), args.worker.as_deref())?;
    let project = targets
        .first()
        .map(|target| target.project.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let server = resolve_server(args.host)?;
    let mut reports = Vec::new();
    for target in targets {
        reports.push(ServerClient::new(&server.host).env(&target.worker).await?);
    }
    if json {
        if reports.len() == 1 {
            print_value(true, &reports[0]);
        } else {
            print_value(
                true,
                &WorkspaceEnvReport {
                    ok: true,
                    project,
                    workers: reports,
                },
            );
        }
    } else {
        for report in &reports {
            for line in dotenv_lines(&project, &report.worker, report, args.qualified) {
                println!("{line}");
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnvTarget {
    project: String,
    worker: String,
}

fn env_targets(
    path: &Path,
    project_filter: Option<&str>,
    worker_filter: Option<&str>,
) -> gumgum_core::Result<Vec<EnvTarget>> {
    let manifest_path = if path.is_dir() {
        path.join("gumgum.toml")
    } else {
        path.to_path_buf()
    };
    let kind = validate_path(&manifest_path)?.manifest_kind;
    let mut targets = match kind {
        ManifestKind::Worker => {
            let manifest = load_worker_path(&manifest_path)?;
            vec![EnvTarget {
                project: manifest
                    .project
                    .map(|project| project.namespace)
                    .unwrap_or_else(|| "root".to_owned()),
                worker: manifest.worker.name,
            }]
        }
        ManifestKind::Workspace => {
            let workspace = load_workspace_path(&manifest_path)?;
            let root = manifest_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut targets = Vec::new();
            for member in workspace.members() {
                let member_path = root.join(member).join("gumgum.toml");
                let manifest = load_worker_path(&member_path)?;
                targets.push(EnvTarget {
                    project: manifest
                        .project
                        .map(|project| project.namespace)
                        .unwrap_or_else(|| workspace.namespace_name().to_owned()),
                    worker: manifest.worker.name,
                });
            }
            targets
        }
    };
    if let Some(project) = project_filter {
        targets.retain(|target| {
            target.project == project || sanitize_name(&target.project) == sanitize_name(project)
        });
    }
    if let Some(worker) = worker_filter {
        targets.retain(|target| {
            target.worker == worker || sanitize_name(&target.worker) == sanitize_name(worker)
        });
    }
    if targets.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            "no environment targets matched this workspace",
        )
        .next_command("gumgum env --project <project>")
        .next_command("gumgum env --worker <worker>")
        .build());
    }
    Ok(targets)
}

fn dotenv_lines(project: &str, worker: &str, report: &EnvReport, qualified: bool) -> Vec<String> {
    report
        .vars
        .iter()
        .map(|var| {
            format!(
                "{}={}",
                dotenv_key(project, worker, &var.name, qualified),
                dotenv_quote(&var.value)
            )
        })
        .collect()
}

fn dotenv_key(project: &str, worker: &str, name: &str, qualified: bool) -> String {
    if qualified {
        format!("{}_{}_{}", env_key(project), env_key(worker), env_key(name))
    } else {
        format!("{}_{}", env_key(worker), env_key(name))
    }
}

fn env_key(value: &str) -> String {
    sanitize_name(value).replace('-', "_").to_ascii_uppercase()
}

fn dotenv_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_api::EnvVar;
    use std::fs;

    #[test]
    fn dotenv_lines_namespace_by_worker_by_default() {
        let report = EnvReport {
            ok: true,
            worker: "api".to_owned(),
            vars: vec![
                EnvVar {
                    name: "DATABASE_URL".to_owned(),
                    value: "postgres://api:g@db.example:5432/api".to_owned(),
                },
                EnvVar {
                    name: "GREETING".to_owned(),
                    value: "hello world".to_owned(),
                },
            ],
            message: "2 environment variable(s)".to_owned(),
        };

        assert_eq!(
            dotenv_lines("visit-counter", "api", &report, false),
            vec![
                "API_DATABASE_URL=postgres://api:g@db.example:5432/api",
                "API_GREETING='hello world'",
            ]
        );
        assert_eq!(
            dotenv_lines("visit-counter", "api", &report, true),
            vec![
                "VISIT_COUNTER_API_DATABASE_URL=postgres://api:g@db.example:5432/api",
                "VISIT_COUNTER_API_GREETING='hello world'",
            ]
        );
    }

    #[test]
    fn env_targets_expand_workspace_and_filter_worker() {
        let dir = temp_dir("workspace");
        fs::create_dir_all(dir.join("api")).unwrap();
        fs::create_dir_all(dir.join("worker")).unwrap();
        fs::write(
            dir.join("gumgum.toml"),
            "[workspace]\nname = \"visit-counter\"\nmembers = [\"api\", \"worker\"]\n",
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

        assert_eq!(
            env_targets(&dir.join("gumgum.toml"), None, Some("api")).unwrap(),
            vec![EnvTarget {
                project: "visit-counter".to_owned(),
                worker: "api".to_owned(),
            }]
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gumgum-env-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
