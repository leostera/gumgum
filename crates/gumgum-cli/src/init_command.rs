use crate::InitArgs;
use gumgum_core::{
    ConfigStore, ErrorCode, GumgumError, InitManifestKind as CoreInitKind, Subsystem,
    default_project_name, init_plan, validate_path,
};
use serde::Serialize;
use std::{fs, path::PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct InitReport {
    ok: bool,
    path: String,
    manifest_kind: &'static str,
    created: bool,
    files: Vec<String>,
    message: String,
}

pub(crate) fn print_init_report(report: &InitReport) {
    println!("{}", report.message);
    println!("kind: {}", report.manifest_kind);
    println!("path: {}", report.path);
    if !report.files.is_empty() {
        println!("Files:");
        for file in &report.files {
            println!("  - {file}");
        }
    }
}

pub(crate) fn init_manifest(args: InitArgs, dry_run: bool) -> gumgum_core::Result<InitReport> {
    let name = args.name.unwrap_or_else(default_project_name);
    init_workspace_manifest(name, args.namespace, args.domain, args.force, dry_run)
}

pub(crate) fn init_workspace_manifest(
    name: String,
    namespace: Option<String>,
    domain: Option<String>,
    force: bool,
    dry_run: bool,
) -> gumgum_core::Result<InitReport> {
    let path = PathBuf::from("gumgum.toml");
    let domain = domain.or_else(|| {
        ConfigStore::from_home_env()
            .and_then(|store| store.load_default_server())
            .ok()
            .flatten()
            .map(|server| server.root_domain)
    });
    let project_name = namespace.unwrap_or_else(|| name.clone());
    let plan = init_plan(
        CoreInitKind::Workspace,
        &project_name,
        &project_name,
        3000,
        &[],
        domain.as_deref(),
    );

    if path.exists() && !force {
        validate_path(&path)?;
        return Ok(InitReport {
            ok: true,
            path: path.display().to_string(),
            manifest_kind: "workspace",
            created: false,
            files: vec![path.display().to_string()],
            message: "gumgum.toml already exists; use --force to overwrite".to_owned(),
        });
    }

    let files = vec![path.display().to_string()];
    if !dry_run {
        fs::write(&path, plan.manifest).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write gumgum.toml",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        validate_path(&path)?;
    }

    Ok(InitReport {
        ok: true,
        path: path.display().to_string(),
        manifest_kind: "workspace",
        created: !dry_run,
        files,
        message: if dry_run {
            "would create workspace gumgum.toml".to_owned()
        } else {
            "created workspace gumgum.toml".to_owned()
        },
    })
}
