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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Copy, Debug)]
pub struct ScaffoldFile {
    pub path: &'static str,
    pub contents: &'static str,
}

pub fn worker_scaffold_files() -> Vec<ScaffoldFile> {
    vec![
        ScaffoldFile {
            path: "Dockerfile",
            contents: r#"FROM python:3.12-alpine
WORKDIR /app
COPY server.py .
ENV PORT=3000
EXPOSE 3000
CMD ["python", "server.py"]
"#,
        },
        ScaffoldFile {
            path: "server.py",
            contents: r#"from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os

PORT = int(os.environ.get("PORT", "3000"))

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok":true}')
            return

        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        body = {"ok": True, "message": "Hello from GumGum.dev"}
        self.wfile.write(json.dumps(body).encode())

    def log_message(self, format, *args):
        print("%s - %s" % (self.address_string(), format % args), flush=True)

HTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
"#,
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitManifestKind {
    Workspace,
    Worker,
}

#[derive(Clone, Debug)]
pub struct InitPlan {
    pub manifest_kind: InitManifestKind,
    pub manifest: String,
    pub scaffold_files: Vec<ScaffoldFile>,
}

pub fn init_plan(
    kind: InitManifestKind,
    name: &str,
    namespace: &str,
    port: u16,
    zones: &[String],
    root_domain: Option<&str>,
) -> InitPlan {
    match kind {
        InitManifestKind::Workspace => InitPlan {
            manifest_kind: kind,
            manifest: workspace_manifest_template(name, root_domain),
            scaffold_files: Vec::new(),
        },
        InitManifestKind::Worker => InitPlan {
            manifest_kind: kind,
            manifest: worker_manifest_template(name, namespace, port, zones),
            scaffold_files: worker_scaffold_files(),
        },
    }
}

pub fn workspace_manifest_template(name: &str, root_domain: Option<&str>) -> String {
    let mut raw = format!("[workspace]\nname = \"{name}\"\nmembers = [\"apps/*\"]\n");
    if let Some(root_domain) = root_domain {
        raw.push_str(&format!("root_domain = \"{root_domain}\"\n"));
    }
    raw
}

pub fn worker_manifest_template(
    name: &str,
    namespace: &str,
    port: u16,
    zones: &[String],
) -> String {
    let mut raw = format!(
        "[project]\nnamespace = \"{namespace}\"\n\n[worker]\nname = \"{name}\"\nbuild_context = \".\"\nport = {port}\nhealth = \"/healthz\"\n"
    );
    for zone in zones {
        raw.push_str(&format!("\n[[zone]]\nname = \"{zone}\"\n"));
    }
    raw
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_template_round_trips_through_validation() {
        let raw = worker_manifest_template("api", "experiments", 8080, &["example.com".to_owned()]);
        let report = validate_str(&raw, "gumgum.toml").expect("worker template validates");
        assert!(report.ok);
        assert_eq!(report.manifest_kind, ManifestKind::Worker);
        let parsed: WorkerManifest = toml::from_str(&raw).expect("worker template parses");
        assert_eq!(parsed.project.unwrap().namespace, "experiments");
        assert_eq!(parsed.worker.name, "api");
        assert_eq!(parsed.worker.port, Some(8080));
        assert_eq!(parsed.zone[0].name, "example.com");
    }

    #[test]
    fn workspace_template_round_trips_through_validation() {
        let raw = workspace_manifest_template("peekaboo", Some("leostera.dev"));
        let report = validate_str(&raw, "gumgum.toml").expect("workspace template validates");
        assert!(report.ok);
        assert_eq!(report.manifest_kind, ManifestKind::Workspace);
        let parsed: WorkspaceManifest = toml::from_str(&raw).expect("workspace template parses");
        assert_eq!(parsed.workspace.name, "peekaboo");
        assert_eq!(
            parsed.workspace.root_domain.as_deref(),
            Some("leostera.dev")
        );
        assert_eq!(parsed.workspace.members, vec!["apps/*"]);
    }

    #[test]
    fn worker_scaffold_contains_health_checked_python_server() {
        let files = worker_scaffold_files();
        assert_eq!(
            files.iter().map(|file| file.path).collect::<Vec<_>>(),
            vec!["Dockerfile", "server.py"]
        );
        let dockerfile = files.iter().find(|file| file.path == "Dockerfile").unwrap();
        assert!(dockerfile.contents.contains("FROM python:3.12-alpine"));
        assert!(
            dockerfile
                .contents
                .contains("CMD [\"python\", \"server.py\"]")
        );
        let server = files.iter().find(|file| file.path == "server.py").unwrap();
        assert!(server.contents.contains("/healthz"));
        assert!(server.contents.contains("Hello from GumGum.dev"));
    }
}
