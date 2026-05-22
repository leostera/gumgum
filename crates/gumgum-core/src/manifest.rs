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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<Namespace>,
    pub workspace: Workspace,
}

impl WorkspaceManifest {
    pub fn namespace_name(&self) -> &str {
        self.namespace
            .as_ref()
            .map(|namespace| namespace.name.as_str())
            .or(self.workspace.namespace.as_deref())
            .or(self.workspace.name.as_deref())
            .unwrap_or("root")
    }

    pub fn root_domain(&self) -> Option<&str> {
        self.namespace
            .as_ref()
            .and_then(|namespace| namespace.root_domain.as_deref())
            .or(self.workspace.root_domain.as_deref())
    }

    pub fn test_domain(&self) -> Option<&str> {
        self.namespace
            .as_ref()
            .and_then(|namespace| namespace.test_domain.as_deref())
            .or(self.workspace.test_domain.as_deref())
    }

    pub fn server(&self) -> Option<&str> {
        self.namespace
            .as_ref()
            .and_then(|namespace| namespace.server.as_deref())
    }

    pub fn members(&self) -> &[String] {
        &self.workspace.members
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Namespace {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Workspace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    pub queue: QueueBindings,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub checks: WorkerChecks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkerChecks {
    #[serde(default = "default_live_check")]
    pub live: String,
    #[serde(default = "default_ready_check")]
    pub ready: String,
}

impl Default for WorkerChecks {
    fn default() -> Self {
        Self {
            live: default_live_check(),
            ready: default_ready_check(),
        }
    }
}

fn default_live_check() -> String {
    "/_/live".to_owned()
}

fn default_ready_check() -> String {
    "/_/ready".to_owned()
}

impl Worker {
    pub fn live_check_path(&self) -> &str {
        &self.checks.live
    }

    pub fn ready_check_path(&self) -> &str {
        if let Some(legacy_health) = self.health.as_deref() {
            legacy_health
        } else {
            &self.checks.ready
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Ingress {
    pub name: String,
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_domain: Option<String>,
    #[serde(default)]
    pub public: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct QueueBindings {
    #[serde(default)]
    pub producer: Vec<QueueBinding>,
    #[serde(default)]
    pub consumer: Vec<QueueBinding>,
}

impl QueueBindings {
    pub fn iter_with_access(&self) -> impl Iterator<Item = (&QueueBinding, &'static str)> {
        self.producer
            .iter()
            .map(|binding| (binding, "write"))
            .chain(self.consumer.iter().map(|binding| (binding, "read")))
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueueBinding {
    pub queue_id: String,
    pub binding: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_id: Option<String>,
    pub binding: Option<String>,
    pub access: Option<String>,
}

impl ObjectBinding {
    pub fn object_id(&self, capability: crate::Capability) -> Option<&str> {
        match capability {
            crate::Capability::Db => self.db_id.as_deref(),
            crate::Capability::Kv => self.kv_id.as_deref(),
            crate::Capability::Blob => self.bucket_id.as_deref(),
            crate::Capability::Queue => self.queue_id.as_deref(),
            crate::Capability::Secret => self.secret_id.as_deref(),
            crate::Capability::Observability | crate::Capability::Manual => None,
        }
    }
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
    let mut raw = format!("[namespace]\nname = \"{name}\"\n");
    if let Some(root_domain) = root_domain {
        raw.push_str(&format!("root_domain = \"{root_domain}\"\n"));
    }
    raw.push_str("\n[workspace]\nmembers = []\n");
    raw
}

pub fn worker_manifest_template(
    name: &str,
    namespace: &str,
    port: u16,
    zones: &[String],
) -> String {
    let mut raw = format!(
        "[project]\nnamespace = \"{namespace}\"\n\n[worker]\nname = \"{name}\"\n\n[worker.checks]\nlive = \"/_/live\"\nready = \"/_/ready\"\n\n[[ingress]]\nname = \"http\"\nprotocol = \"http\"\nport = {port}\n"
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

    if value.get("workspace").is_some() || value.get("namespace").is_some() {
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
    if manifest.namespace_name().trim().is_empty() {
        return Err(ManifestError::Validation(
            "namespace.name must not be empty".to_owned(),
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
    validate_object_bindings(crate::Capability::Db, &manifest.database, "database")?;
    validate_object_bindings(crate::Capability::Kv, &manifest.kv, "kv")?;
    validate_object_bindings(crate::Capability::Blob, &manifest.bucket, "bucket")?;
    validate_queue_bindings(&manifest.queue)?;
    Ok(())
}

fn validate_queue_bindings(bindings: &QueueBindings) -> std::result::Result<(), ManifestError> {
    for (binding, role) in bindings.iter_with_access() {
        if binding.queue_id.trim().is_empty() {
            return Err(ManifestError::Validation(format!(
                "queue.{role} binding must declare queue_id"
            )));
        }
        if binding.binding.trim().is_empty() {
            return Err(ManifestError::Validation(format!(
                "queue.{role} binding must declare binding"
            )));
        }
    }
    Ok(())
}

fn validate_object_bindings(
    capability: crate::Capability,
    bindings: &[ObjectBinding],
    table: &str,
) -> std::result::Result<(), ManifestError> {
    for binding in bindings {
        if binding
            .object_id(capability)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .is_none()
        {
            return Err(ManifestError::Validation(format!(
                "{table} binding must declare {}_id",
                match capability {
                    crate::Capability::Db => "db",
                    crate::Capability::Kv => "kv",
                    crate::Capability::Blob => "bucket",
                    crate::Capability::Queue => "queue",
                    crate::Capability::Secret => "secret",
                    crate::Capability::Observability | crate::Capability::Manual => "object",
                }
            )));
        }
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
        assert_eq!(parsed.worker.port, None);
        assert_eq!(parsed.ingress[0].port, Some(8080));
        assert_eq!(parsed.zone[0].name, "example.com");
    }

    #[test]
    fn workspace_template_round_trips_through_validation() {
        let raw = workspace_manifest_template("peekaboo", Some("leostera.dev"));
        let report = validate_str(&raw, "gumgum.toml").expect("workspace template validates");
        assert!(report.ok);
        assert_eq!(report.manifest_kind, ManifestKind::Workspace);
        let parsed: WorkspaceManifest = toml::from_str(&raw).expect("workspace template parses");
        assert_eq!(parsed.namespace_name(), "peekaboo");
        assert_eq!(parsed.root_domain(), Some("leostera.dev"));
        assert!(parsed.workspace.members.is_empty());
    }

    #[test]
    fn workspace_manifest_supports_namespace_metadata_and_legacy_shape() {
        let modern = r#"[namespace]
name = "visit-counter"
root_domain = "example.dev"
test_domain = "example.test"
server = "isolated"

[workspace]
members = ["api", "worker"]
"#;
        let parsed: WorkspaceManifest = toml::from_str(modern).unwrap();
        assert_eq!(parsed.namespace_name(), "visit-counter");
        assert_eq!(parsed.root_domain(), Some("example.dev"));
        assert_eq!(parsed.test_domain(), Some("example.test"));
        assert_eq!(parsed.server(), Some("isolated"));
        assert_eq!(parsed.members(), &["api".to_owned(), "worker".to_owned()]);

        let legacy = r#"[workspace]
name = "visit-counter"
root_domain = "example.dev"
members = ["api"]
"#;
        let parsed: WorkspaceManifest = toml::from_str(legacy).unwrap();
        assert_eq!(parsed.namespace_name(), "visit-counter");
        assert_eq!(parsed.root_domain(), Some("example.dev"));
        assert_eq!(parsed.members(), &["api".to_owned()]);
    }

    #[test]
    fn object_bindings_reject_user_supplied_dns() {
        let raw = r#"[worker]
name = "api"
build_context = "."

[[kv]]
name = "user-counters"
binding = "USER_COUNTERS"
dns = "user-counters.kv.example.dev"
"#;
        let error = validate_str(raw, "gumgum.toml").unwrap_err().to_string();
        assert!(error.contains("unknown field `dns`"));
    }

    #[test]
    fn queue_bindings_use_producer_consumer_roles() {
        let raw = r#"[worker]
name = "api"
build_context = "."

[[queue.producer]]
queue_id = "visit-events"
binding = "VISIT_EVENTS_QUEUE"

[[queue.consumer]]
queue_id = "visit-events"
binding = "VISIT_EVENTS_QUEUE"
"#;
        let report = validate_str(raw, "gumgum.toml").expect("queue roles validate");
        assert!(report.ok);
        let parsed: WorkerManifest = toml::from_str(raw).expect("queue roles parse");
        assert_eq!(parsed.queue.producer[0].queue_id, "visit-events");
        assert_eq!(parsed.queue.consumer[0].binding, "VISIT_EVENTS_QUEUE");
    }

    #[test]
    fn queue_bindings_require_binding_name() {
        let raw = r#"[worker]
name = "api"
build_context = "."

[[queue.consumer]]
queue_id = "visit-events"
"#;
        let error = validate_str(raw, "gumgum.toml").unwrap_err().to_string();
        assert!(error.contains("missing field `binding`"));
    }

    #[test]
    fn queue_bindings_reject_access_field() {
        let raw = r#"[worker]
name = "api"
build_context = "."

[[queue.producer]]
queue_id = "visit-events"
binding = "VISIT_EVENTS_QUEUE"
access = "write"
"#;
        let error = validate_str(raw, "gumgum.toml").unwrap_err().to_string();
        assert!(error.contains("unknown field `access`"));
    }

    #[test]
    fn init_plan_includes_scaffold_only_for_workers() {
        let worker = init_plan(
            InitManifestKind::Worker,
            "api",
            "experiments",
            8080,
            &[],
            None,
        );
        assert_eq!(worker.manifest_kind, InitManifestKind::Worker);
        assert!(worker.manifest.contains("[worker]"));
        assert_eq!(worker.scaffold_files.len(), 2);

        let workspace = init_plan(
            InitManifestKind::Workspace,
            "peekaboo",
            "ignored",
            3000,
            &[],
            Some("leostera.dev"),
        );
        assert_eq!(workspace.manifest_kind, InitManifestKind::Workspace);
        assert!(workspace.manifest.contains("[namespace]"));
        assert!(workspace.manifest.contains("[workspace]"));
        assert!(workspace.scaffold_files.is_empty());
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
