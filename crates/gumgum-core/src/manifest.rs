use crate::{ErrorCode, GumgumError, Result, Subsystem};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("manifest validation failed: {0}")]
    Validation(String),
}

impl From<ManifestError> for GumgumError {
    fn from(value: ManifestError) -> Self {
        match value {
            ManifestError::Read { path, source } => GumgumError::structured(
                Subsystem::Manifest,
                ErrorCode::ManifestNotFound,
                format!("could not read manifest at {path}"),
            )
            .likely_cause(source.to_string())
            .next_command("gumgum init")
            .build(),
            ManifestError::Parse { path, source } => GumgumError::structured(
                Subsystem::Manifest,
                ErrorCode::ManifestParseFailed,
                format!("could not parse manifest at {path}"),
            )
            .likely_cause(source.to_string())
            .next_command(format!("gumgum schema validate {path}"))
            .build(),
            ManifestError::Validation(message) => GumgumError::structured(
                Subsystem::Schema,
                ErrorCode::ManifestValidationFailed,
                message,
            )
            .next_command("gumgum schema explain")
            .build(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceManifest {
    pub workspace: Workspace,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Workspace {
    pub name: String,
    pub namespace: Option<String>,
    pub root_domain: Option<String>,
    pub test_domain: Option<String>,
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkerManifest {
    #[serde(default)]
    pub project: Option<Project>,
    pub worker: Worker,
    #[serde(default)]
    pub zone: Vec<Zone>,
    #[serde(default)]
    pub ingress: Vec<Ingress>,
    #[serde(default)]
    pub database: Vec<ObjectBinding>,
    #[serde(default)]
    pub kv: Vec<ObjectBinding>,
    #[serde(default)]
    pub bucket: Vec<ObjectBinding>,
    #[serde(default)]
    pub observability: Option<Observability>,
    #[serde(default)]
    pub limits: Option<Limits>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
    pub namespace: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Zone {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Worker {
    pub name: String,
    pub image: Option<String>,
    pub build_context: Option<String>,
    pub command: Option<String>,
    pub port: Option<u16>,
    pub health: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Ingress {
    pub name: String,
    pub protocol: String,
    pub local_domain: String,
    pub public_domain: Option<String>,
    #[serde(default)]
    pub publish: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ObjectBinding {
    pub name: String,
    pub binding: Option<String>,
    pub access: Option<String>,
    pub dns: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Observability {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Limits {
    pub cpus: Option<f32>,
    pub memory_mb: Option<u32>,
    pub pids: Option<u32>,
    pub restart: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub path: String,
    pub manifest_kind: ManifestKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    Workspace,
    Worker,
}

pub fn validate_path(path: &Path) -> Result<ValidationReport> {
    let raw = fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.display().to_string(),
        source,
    })?;

    validate_str(&raw, &path.display().to_string()).map_err(Into::into)
}

pub fn load_worker_path(path: &Path) -> Result<WorkerManifest> {
    let raw = fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let manifest: WorkerManifest = toml::from_str(&raw).map_err(|source| ManifestError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    validate_worker(&manifest)?;
    Ok(manifest)
}

pub fn load_workspace_path(path: &Path) -> Result<WorkspaceManifest> {
    let raw = fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let manifest: WorkspaceManifest =
        toml::from_str(&raw).map_err(|source| ManifestError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    validate_workspace(&manifest)?;
    Ok(manifest)
}

pub fn validate_str(raw: &str, path: &str) -> std::result::Result<ValidationReport, ManifestError> {
    let value: toml::Value = toml::from_str(raw).map_err(|source| ManifestError::Parse {
        path: path.to_owned(),
        source,
    })?;

    if value.get("workspace").is_some() {
        let manifest: WorkspaceManifest =
            toml::from_str(raw).map_err(|source| ManifestError::Parse {
                path: path.to_owned(),
                source,
            })?;
        validate_workspace(&manifest)?;
        return Ok(ValidationReport {
            ok: true,
            path: path.to_owned(),
            manifest_kind: ManifestKind::Workspace,
            message: "workspace manifest is valid".to_owned(),
        });
    }

    if value.get("worker").is_some() {
        let manifest: WorkerManifest =
            toml::from_str(raw).map_err(|source| ManifestError::Parse {
                path: path.to_owned(),
                source,
            })?;
        validate_worker(&manifest)?;
        return Ok(ValidationReport {
            ok: true,
            path: path.to_owned(),
            manifest_kind: ManifestKind::Worker,
            message: "worker manifest is valid".to_owned(),
        });
    }

    Err(ManifestError::Validation(
        "manifest must contain either [workspace] or [worker]".to_owned(),
    ))
}

fn validate_workspace(manifest: &WorkspaceManifest) -> std::result::Result<(), ManifestError> {
    if manifest.workspace.name.trim().is_empty() {
        return Err(ManifestError::Validation(
            "workspace.name must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_worker(manifest: &WorkerManifest) -> std::result::Result<(), ManifestError> {
    if manifest.worker.name.trim().is_empty() {
        return Err(ManifestError::Validation(
            "worker.name must not be empty".to_owned(),
        ));
    }
    if let Some(project) = &manifest.project {
        if project.namespace.trim().is_empty() {
            return Err(ManifestError::Validation(
                "project.namespace must not be empty".to_owned(),
            ));
        }
    }
    for zone in &manifest.zone {
        if zone.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                "zone.name must not be empty".to_owned(),
            ));
        }
    }
    if manifest.worker.image.is_none() && manifest.worker.build_context.is_none() {
        return Err(ManifestError::Validation(
            "worker.image or worker.build_context is required".to_owned(),
        ));
    }
    Ok(())
}
