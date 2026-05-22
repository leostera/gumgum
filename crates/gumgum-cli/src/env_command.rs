use crate::{EnvArgs, print_value, resolve_server};
use gumgum_api::{EnvReport, EnvVar};
use gumgum_core::{
    Capability, ErrorCode, GumgumError, ManifestKind, ObjectBinding, Subsystem, load_worker_path,
    load_workspace_path, projected_binding_env, sanitize_name, validate_path,
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
        let mut report = ServerClient::new(&server.host).env(&target.worker).await?;
        if report.vars.is_empty() && !target.local_vars.is_empty() {
            report.vars = target.local_vars;
            report.message = format!(
                "{} environment variable(s) from gumgum.toml",
                report.vars.len()
            );
        }
        reports.push(report);
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

#[derive(Clone, Debug)]
struct EnvTarget {
    project: String,
    worker: String,
    local_vars: Vec<EnvVar>,
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
            let project = manifest
                .project
                .as_ref()
                .map(|project| project.namespace.clone())
                .unwrap_or_else(|| "root".to_owned());
            vec![EnvTarget {
                worker: manifest.worker.name.clone(),
                local_vars: manifest_env_vars(&manifest),
                project,
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
                        .as_ref()
                        .map(|project| project.namespace.clone())
                        .unwrap_or_else(|| workspace.namespace_name().to_owned()),
                    worker: manifest.worker.name.clone(),
                    local_vars: manifest_env_vars(&manifest),
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

fn manifest_env_vars(manifest: &gumgum_core::WorkerManifest) -> Vec<EnvVar> {
    let mut vars = Vec::new();
    extend_manifest_binding_env(&mut vars, Capability::Db, &manifest.database);
    extend_manifest_binding_env(&mut vars, Capability::Kv, &manifest.kv);
    extend_manifest_binding_env(&mut vars, Capability::Blob, &manifest.bucket);
    for (binding, _access) in manifest.queue.iter_with_access() {
        for (name, value) in
            projected_binding_env(Capability::Queue, &binding.binding, &binding.queue_id)
        {
            vars.push(EnvVar { name, value });
        }
    }
    vars.sort_by(|left, right| left.name.cmp(&right.name));
    vars
}

fn extend_manifest_binding_env(
    vars: &mut Vec<EnvVar>,
    capability: Capability,
    bindings: &[ObjectBinding],
) {
    for binding in bindings {
        let Some(env_name) = binding.binding.as_deref() else {
            continue;
        };
        let Some(object_id) = binding.object_id(capability) else {
            continue;
        };
        for (name, value) in projected_binding_env(capability, env_name, object_id) {
            vars.push(EnvVar { name, value });
        }
    }
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
    fn manifest_env_vars_project_worker_bindings_before_deploy() {
        let raw = r#"[worker]
name = "api"

[[kv]]
kv_id = "user-counters"
binding = "USER_COUNTERS"
access = "read-write"

[[bucket]]
bucket_id = "visit-requests"
binding = "VISIT_REQUESTS_BUCKET"
access = "read-write"

[[queue.producer]]
queue_id = "visit-events"
binding = "VISIT_EVENTS_QUEUE"
"#;
        let manifest: gumgum_core::WorkerManifest = toml::from_str(raw).unwrap();
        let vars = manifest_env_vars(&manifest);
        let names = vars.iter().map(|var| var.name.as_str()).collect::<Vec<_>>();
        assert!(names.contains(&"USER_COUNTERS"));
        assert!(names.contains(&"VISIT_REQUESTS_BUCKET"));
        assert!(names.contains(&"VISIT_EVENTS_QUEUE"));
        assert!(names.contains(&"VISIT_EVENTS_QUEUE_TOPIC"));
    }

    #[test]
    fn env_targets_expand_workspace_and_filter_worker() {
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

        let targets = env_targets(&dir.join("gumgum.toml"), None, Some("api")).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].project, "visit-counter");
        assert_eq!(targets[0].worker, "api");
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
