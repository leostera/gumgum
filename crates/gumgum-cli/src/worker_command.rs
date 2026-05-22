use crate::{
    WorkerArgs, WorkerCommand, WorkerCreateArgs, WorkerDeleteArgs, WorkerListArgs, print_value,
};
use gumgum_core::{
    ConfigStore, ErrorCode, GumgumError, InitManifestKind, ScaffoldFile, Subsystem, init_plan,
    load_worker_path, load_workspace_path, validate_path,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub(crate) struct WorkerCommandReport {
    ok: bool,
    action: String,
    workers: Vec<WorkerEntry>,
    files: Vec<String>,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct WorkerEntry {
    name: String,
    path: String,
    port: Option<u16>,
}

pub(crate) fn worker_command(
    args: WorkerArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let report = match args.command {
        WorkerCommand::Create(args) => create_worker(args, dry_run)?,
        WorkerCommand::List(args) => list_workers(args)?,
        WorkerCommand::Delete(args) => delete_worker(args, dry_run)?,
    };
    if json {
        print_value(true, &report);
    } else {
        print_worker_report(&report);
    }
    Ok(())
}

fn create_worker(
    args: WorkerCreateArgs,
    dry_run: bool,
) -> gumgum_core::Result<WorkerCommandReport> {
    let workspace_path = PathBuf::from("gumgum.toml");
    let workspace = load_workspace_path(&workspace_path)?;
    let worker_dir = args.dir.unwrap_or_else(|| PathBuf::from(&args.name));
    let manifest_path = worker_dir.join("gumgum.toml");
    if manifest_path.exists() && !args.force {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!(
                "worker manifest already exists at {}",
                manifest_path.display()
            ),
        )
        .next_command(format!("gumgum worker create {} --force", args.name))
        .build());
    }
    let namespace = args
        .namespace
        .clone()
        .unwrap_or_else(|| workspace.namespace_name().to_owned());
    let plan = init_plan(
        InitManifestKind::Worker,
        &args.name,
        &namespace,
        args.port,
        &args.zones,
        workspace.root_domain(),
    );
    let mut files = vec![manifest_path.display().to_string()];
    files.extend(
        plan.scaffold_files
            .iter()
            .map(|file| worker_dir.join(file.path).display().to_string()),
    );
    let member = worker_dir.display().to_string();
    if !dry_run {
        fs::create_dir_all(&worker_dir).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not create worker directory",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        fs::write(&manifest_path, plan.manifest).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write worker gumgum.toml",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        write_scaffold_files(&worker_dir, &plan.scaffold_files)?;
        validate_path(&manifest_path)?;
        save_workspace_members(&workspace_path, &member, None)?;
    }
    Ok(WorkerCommandReport {
        ok: true,
        action: "create".to_owned(),
        workers: vec![WorkerEntry {
            name: args.name,
            path: manifest_path.display().to_string(),
            port: Some(args.port),
        }],
        files,
        message: if dry_run {
            "worker create preview; no files changed".to_owned()
        } else {
            "worker created".to_owned()
        },
    })
}

fn list_workers(args: WorkerListArgs) -> gumgum_core::Result<WorkerCommandReport> {
    let workspace = load_workspace_path(&args.workspace)?;
    let base = args
        .workspace
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut workers = Vec::new();
    for member in workspace.workspace.members {
        if member.contains('*') {
            continue;
        }
        let manifest_path = base.join(&member).join("gumgum.toml");
        if let Ok(manifest) = load_worker_path(&manifest_path) {
            workers.push(WorkerEntry {
                name: manifest.worker.name,
                path: manifest_path.display().to_string(),
                port: manifest.worker.port,
            });
        }
    }
    workers.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(WorkerCommandReport {
        ok: true,
        action: "list".to_owned(),
        files: Vec::new(),
        message: format!("{} worker(s)", workers.len()),
        workers,
    })
}

fn delete_worker(
    args: WorkerDeleteArgs,
    dry_run: bool,
) -> gumgum_core::Result<WorkerCommandReport> {
    let workspace = load_workspace_path(&args.workspace)?;
    let base = args
        .workspace
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut matched_member = None;
    let mut matched_entry = None;
    for member in &workspace.workspace.members {
        if member.contains('*') {
            continue;
        }
        let manifest_path = base.join(member).join("gumgum.toml");
        if let Ok(manifest) = load_worker_path(&manifest_path) {
            if manifest.worker.name == args.name || member == &args.name {
                matched_member = Some(member.clone());
                matched_entry = Some(WorkerEntry {
                    name: manifest.worker.name,
                    path: manifest_path.display().to_string(),
                    port: manifest.worker.port,
                });
                break;
            }
        }
    }
    let member = matched_member.ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!("unknown worker {}", args.name),
        )
        .next_command("gumgum worker list")
        .build()
    })?;
    let entry = matched_entry.clone().expect("matched worker entry exists");
    let graph_path = ConfigStore::from_home_env()?.root().join("graph.sqlite");
    let active_bindings = gumgum_core::GraphStore::new(graph_path).worker_bindings(&entry.name)?;
    if !active_bindings.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!("worker {} has active bindings", entry.name),
        )
        .likely_cause(
            active_bindings
                .iter()
                .map(|binding| format!("{} as {}", binding.worker, binding.binding))
                .collect::<Vec<_>>()
                .join(", "),
        )
        .next_command(format!(
            "gumgum db unbind <name> --to {} --as <binding>",
            entry.name
        ))
        .next_command(format!(
            "gumgum kv unbind <name> --to {} --as <binding>",
            entry.name
        ))
        .next_command(format!(
            "gumgum bucket unbind <name> --to {} --as <binding>",
            entry.name
        ))
        .next_command(format!(
            "gumgum queue unbind <name> --to {} --as <binding>",
            entry.name
        ))
        .build());
    }
    if !dry_run {
        save_workspace_members(&args.workspace, "", Some(&member))?;
    }
    Ok(WorkerCommandReport {
        ok: true,
        action: "delete".to_owned(),
        workers: matched_entry.into_iter().collect(),
        files: Vec::new(),
        message: if dry_run {
            "worker delete preview; no workspace changed and no source files removed".to_owned()
        } else {
            "worker removed from workspace; source files left in place".to_owned()
        },
    })
}

fn save_workspace_members(
    workspace_path: &PathBuf,
    add_member: &str,
    remove_member: Option<&str>,
) -> gumgum_core::Result<()> {
    let mut workspace = load_workspace_path(workspace_path)?;
    if let Some(remove_member) = remove_member {
        workspace
            .workspace
            .members
            .retain(|member| member != remove_member);
    }
    if !add_member.is_empty()
        && !workspace
            .workspace
            .members
            .iter()
            .any(|member| member == add_member)
    {
        workspace.workspace.members.push(add_member.to_owned());
    }
    workspace.workspace.members.sort();
    let raw = toml::to_string_pretty(&workspace).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not serialize workspace manifest",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    fs::write(workspace_path, raw).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not write workspace manifest",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    Ok(())
}

fn write_scaffold_files(worker_dir: &Path, files: &[ScaffoldFile]) -> gumgum_core::Result<()> {
    for file in files {
        let path = worker_dir.join(file.path);
        if path.exists() {
            continue;
        }
        fs::write(&path, file.contents).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                format!("could not write {}", path.display()),
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    }
    Ok(())
}

fn worker_report_lines(report: &WorkerCommandReport) -> Vec<String> {
    let mut lines = vec![report.message.clone()];
    if !report.workers.is_empty() {
        lines.push(format!("{:<24} {:<8} PATH", "WORKER", "PORT"));
    }
    lines.extend(report.workers.iter().map(|worker| {
        format!(
            "{:<24} {:<8} {}",
            worker.name,
            worker
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            worker.path
        )
    }));
    lines.extend(report.files.iter().map(|file| format!("  - {file}")));
    lines
}

fn print_worker_report(report: &WorkerCommandReport) {
    for line in worker_report_lines(report) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{load_workspace_path, worker_manifest_template, workspace_manifest_template};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gumgum-worker-{name}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn worker_report_lines_include_table_header() {
        let report = WorkerCommandReport {
            ok: true,
            action: "list".to_owned(),
            workers: vec![WorkerEntry {
                name: "api".to_owned(),
                path: "api/gumgum.toml".to_owned(),
                port: Some(3000),
            }],
            files: Vec::new(),
            message: "1 worker(s)".to_owned(),
        };

        let lines = worker_report_lines(&report);
        assert_eq!(lines[0], "1 worker(s)");
        assert_eq!(lines[1], format!("{:<24} {:<8} PATH", "WORKER", "PORT"));
        assert!(lines[2].contains("api/gumgum.toml"));
    }

    #[test]
    fn workspace_member_mutation_adds_sorts_and_removes_without_source_delete() {
        let root = temp_dir("members");
        let workspace_path = root.join("gumgum.toml");
        fs::write(
            &workspace_path,
            workspace_manifest_template("visit-counter", Some("example.test")),
        )
        .unwrap();
        let api_dir = root.join("api");
        fs::create_dir_all(&api_dir).unwrap();
        fs::write(api_dir.join("server.py"), "print('keep me')\n").unwrap();

        save_workspace_members(&workspace_path, "worker", None).unwrap();
        save_workspace_members(&workspace_path, "api", None).unwrap();
        save_workspace_members(&workspace_path, "api", None).unwrap();
        assert_eq!(
            load_workspace_path(&workspace_path)
                .unwrap()
                .workspace
                .members,
            vec!["api".to_owned(), "worker".to_owned()]
        );

        save_workspace_members(&workspace_path, "", Some("api")).unwrap();
        assert_eq!(
            load_workspace_path(&workspace_path)
                .unwrap()
                .workspace
                .members,
            vec!["worker".to_owned()]
        );
        assert!(api_dir.join("server.py").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_workers_reads_workspace_members_and_ignores_globs_or_missing_manifests() {
        let root = temp_dir("list");
        let workspace_path = root.join("gumgum.toml");
        fs::write(
            &workspace_path,
            "[workspace]\nname = \"visit-counter\"\nmembers = [\"worker\", \"api\", \"crates/*\"]\n",
        )
        .unwrap();
        for (dir, name, port) in [("api", "api", 3000), ("worker", "worker", 3001)] {
            let worker_dir = root.join(dir);
            fs::create_dir_all(&worker_dir).unwrap();
            fs::write(
                worker_dir.join("gumgum.toml"),
                worker_manifest_template(name, "visit-counter", port, &[]),
            )
            .unwrap();
        }

        let report = list_workers(WorkerListArgs {
            workspace: workspace_path,
        })
        .unwrap();
        assert_eq!(report.message, "2 worker(s)");
        assert_eq!(
            report
                .workers
                .iter()
                .map(|worker| worker.name.as_str())
                .collect::<Vec<_>>(),
            vec!["api", "worker"]
        );
        let _ = fs::remove_dir_all(root);
    }
}
