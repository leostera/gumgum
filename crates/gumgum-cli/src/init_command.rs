use crate::{InitArgs, InitKind};
use gumgum_core::{
    ConfigStore, ErrorCode, GumgumError, InitManifestKind as CoreInitKind, ScaffoldFile, Subsystem,
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

pub(crate) fn init_manifest(args: InitArgs, dry_run: bool) -> gumgum_core::Result<InitReport> {
    let path = PathBuf::from("gumgum.toml");
    let name = args.name.unwrap_or_else(default_project_name);
    let root_domain = args.root_domain.or_else(|| {
        ConfigStore::from_home_env()
            .and_then(|store| store.load_default_server())
            .ok()
            .flatten()
            .map(|server| server.root_domain)
    });
    let namespace = args.namespace.unwrap_or_else(|| name.clone());
    let plan = init_plan(
        match args.kind {
            InitKind::Workspace => CoreInitKind::Workspace,
            InitKind::Worker => CoreInitKind::Worker,
        },
        &name,
        &namespace,
        args.port,
        &args.zones,
        root_domain.as_deref(),
    );

    if path.exists() && !args.force {
        validate_path(&path)?;
        return Ok(InitReport {
            ok: true,
            path: path.display().to_string(),
            manifest_kind: match args.kind {
                InitKind::Workspace => "workspace",
                InitKind::Worker => "worker",
            },
            created: false,
            files: vec![path.display().to_string()],
            message: "gumgum.toml already exists; use --force to overwrite".to_owned(),
        });
    }

    let mut files = vec![path.display().to_string()];
    if matches!(args.kind, InitKind::Worker) {
        files.extend(scaffold_example_files(&plan.scaffold_files, dry_run)?);
    }

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
        manifest_kind: match args.kind {
            InitKind::Workspace => "workspace",
            InitKind::Worker => "worker",
        },
        created: !dry_run,
        files,
        message: if dry_run {
            "would create gumgum.toml".to_owned()
        } else {
            "created gumgum.toml".to_owned()
        },
    })
}

fn scaffold_example_files(
    files: &[ScaffoldFile],
    dry_run: bool,
) -> gumgum_core::Result<Vec<String>> {
    let paths = files.iter().map(|file| file.path.to_owned()).collect();
    if dry_run {
        return Ok(paths);
    }

    for file in files {
        write_if_missing(file.path, file.contents)?;
    }
    Ok(paths)
}

fn write_if_missing(path: &str, contents: &str) -> gumgum_core::Result<()> {
    if PathBuf::from(path).exists() {
        return Ok(());
    }
    fs::write(path, contents).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            format!("could not write {path}"),
        )
        .likely_cause(source.to_string())
        .build()
    })
}
