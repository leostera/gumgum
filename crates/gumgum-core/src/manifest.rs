use crate::{ErrorCode, ErrorKind, GumgumError, Result, Subsystem};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest.read_failed")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest.parse_failed")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("manifest.validation_failed")]
    Validation(ManifestValidationIssue),
}

impl From<ManifestError> for GumgumError {
    fn from(value: ManifestError) -> Self {
        match value {
            ManifestError::Read { path, source } => GumgumError::structured_kind(
                Subsystem::Manifest,
                ErrorCode::ManifestNotFound,
                ErrorKind::ManifestReadFailed,
            )
            .likely_cause(format!("{path}: {source}"))
            .next_command("gumgum init")
            .build(),
            ManifestError::Parse { path, source } => GumgumError::structured_kind(
                Subsystem::Manifest,
                ErrorCode::ManifestParseFailed,
                ErrorKind::ManifestParseFailed,
            )
            .likely_cause(format!("{path}: {source}"))
            .next_command(format!("gumgum schema validate {path}"))
            .build(),
            ManifestError::Validation(issue) => GumgumError::structured_kind(
                Subsystem::Schema,
                ErrorCode::ManifestValidationFailed,
                ErrorKind::ManifestValidationFailed,
            )
            .likely_cause(issue.machine_code())
            .next_command("gumgum schema explain")
            .build(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceManifest {
    pub project: WorkspaceProject,
    pub workspace: Workspace,
}

impl WorkspaceManifest {
    pub fn project_name(&self) -> &str {
        &self.project.name
    }

    pub fn namespace_name(&self) -> &str {
        self.project_name()
    }

    pub fn domain(&self) -> &str {
        &self.project.domain
    }

    pub fn server(&self) -> Option<&str> {
        self.project.server.as_deref()
    }

    pub fn members(&self) -> &[String] {
        &self.workspace.members
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceProject {
    pub name: String,
    pub domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Workspace {
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
    #[serde(default, rename = "secret")]
    pub secrets: Vec<SecretBinding>,
    #[serde(default)]
    pub observability: Option<Observability>,
    #[serde(default, rename = "dashboard")]
    pub dashboards: Vec<Dashboard>,
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
    pub record: Option<String>,
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

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretBinding {
    pub name: String,
    pub binding: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observability {
    #[serde(default, alias = "enabled")]
    pub enable: bool,
    #[serde(default = "default_prometheus_metrics")]
    pub prometheus_metrics: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grafana: Option<GrafanaObservability>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrafanaObservability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dashboard {
    pub name: String,
    pub path: String,
}

fn default_prometheus_metrics() -> String {
    "/_/metrics".to_owned()
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
    pub status: ValidationStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Valid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestKind {
    Workspace,
    Worker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestValidationIssue {
    ManifestKindMissing,
    ProjectNameEmpty,
    ProjectNameIsDomain,
    ProjectDomainEmpty,
    WorkerNameEmpty,
    ProjectNamespaceEmpty,
    ZoneNameEmpty,
    SecretNameMissing,
    SecretBindingMissing,
    PrometheusMetricsPathRelative,
    GrafanaSourcesEmpty,
    DashboardNameMissing,
    DashboardPathMissing,
    QueueIdMissing,
    QueueBindingMissing,
    ObjectIdMissing,
}

impl ManifestValidationIssue {
    pub fn machine_code(self) -> &'static str {
        match self {
            ManifestValidationIssue::ManifestKindMissing => "manifest.kind.missing",
            ManifestValidationIssue::ProjectNameEmpty => "project.name.empty",
            ManifestValidationIssue::ProjectNameIsDomain => "project.name.is_domain",
            ManifestValidationIssue::ProjectDomainEmpty => "project.domain.empty",
            ManifestValidationIssue::WorkerNameEmpty => "worker.name.empty",
            ManifestValidationIssue::ProjectNamespaceEmpty => "project.namespace.empty",
            ManifestValidationIssue::ZoneNameEmpty => "zone.name.empty",
            ManifestValidationIssue::SecretNameMissing => "secret.name.missing",
            ManifestValidationIssue::SecretBindingMissing => "secret.binding.missing",
            ManifestValidationIssue::PrometheusMetricsPathRelative => {
                "observability.prometheus_metrics.relative"
            }
            ManifestValidationIssue::GrafanaSourcesEmpty => "observability.grafana.sources.empty",
            ManifestValidationIssue::DashboardNameMissing => "dashboard.name.missing",
            ManifestValidationIssue::DashboardPathMissing => "dashboard.path.missing",
            ManifestValidationIssue::QueueIdMissing => "queue.id.missing",
            ManifestValidationIssue::QueueBindingMissing => "queue.binding.missing",
            ManifestValidationIssue::ObjectIdMissing => "object.id.missing",
        }
    }
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
    domain: Option<&str>,
) -> InitPlan {
    match kind {
        InitManifestKind::Workspace => InitPlan {
            manifest_kind: kind,
            manifest: workspace_manifest_template(name, domain),
            scaffold_files: Vec::new(),
        },
        InitManifestKind::Worker => InitPlan {
            manifest_kind: kind,
            manifest: worker_manifest_template(name, namespace, port, zones),
            scaffold_files: worker_scaffold_files(),
        },
    }
}

pub fn workspace_manifest_template(name: &str, domain: Option<&str>) -> String {
    let domain = domain.unwrap_or("example.com");
    format!("[project]\nname = \"{name}\"\ndomain = \"{domain}\"\n\n[workspace]\nmembers = []\n")
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
            status: ValidationStatus::Valid,
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
            status: ValidationStatus::Valid,
        });
    }

    Err(ManifestError::Validation(
        ManifestValidationIssue::ManifestKindMissing,
    ))
}

fn validate_workspace(manifest: &WorkspaceManifest) -> std::result::Result<(), ManifestError> {
    if manifest.project.name.trim().is_empty() {
        return Err(ManifestError::Validation(
            ManifestValidationIssue::ProjectNameEmpty,
        ));
    }
    if manifest.project.name.contains('.') {
        return Err(ManifestError::Validation(
            ManifestValidationIssue::ProjectNameIsDomain,
        ));
    }
    if manifest.project.domain.trim().is_empty() {
        return Err(ManifestError::Validation(
            ManifestValidationIssue::ProjectDomainEmpty,
        ));
    }
    Ok(())
}

fn validate_worker(manifest: &WorkerManifest) -> std::result::Result<(), ManifestError> {
    if manifest.worker.name.trim().is_empty() {
        return Err(ManifestError::Validation(
            ManifestValidationIssue::WorkerNameEmpty,
        ));
    }
    if let Some(project) = &manifest.project {
        if project.namespace.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::ProjectNamespaceEmpty,
            ));
        }
    }
    for zone in &manifest.zone {
        if zone.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::ZoneNameEmpty,
            ));
        }
    }
    validate_object_bindings(crate::Capability::Db, &manifest.database, "database")?;
    validate_object_bindings(crate::Capability::Kv, &manifest.kv, "kv")?;
    validate_object_bindings(crate::Capability::Blob, &manifest.bucket, "bucket")?;
    validate_queue_bindings(&manifest.queue)?;
    validate_secret_bindings(&manifest.secrets)?;
    validate_observability(manifest.observability.as_ref())?;
    validate_dashboards(&manifest.dashboards)?;
    Ok(())
}

fn validate_secret_bindings(bindings: &[SecretBinding]) -> std::result::Result<(), ManifestError> {
    for secret in bindings {
        if secret.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::SecretNameMissing,
            ));
        }
        if secret.binding.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::SecretBindingMissing,
            ));
        }
    }
    Ok(())
}

fn validate_observability(
    observability: Option<&Observability>,
) -> std::result::Result<(), ManifestError> {
    let Some(observability) = observability else {
        return Ok(());
    };
    if observability.enable && !observability.prometheus_metrics.starts_with('/') {
        return Err(ManifestError::Validation(
            ManifestValidationIssue::PrometheusMetricsPathRelative,
        ));
    }
    if let Some(grafana) = &observability.grafana {
        if matches!(grafana.sources.as_deref(), Some(path) if path.trim().is_empty()) {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::GrafanaSourcesEmpty,
            ));
        }
    }
    Ok(())
}

fn validate_dashboards(dashboards: &[Dashboard]) -> std::result::Result<(), ManifestError> {
    for dashboard in dashboards {
        if dashboard.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::DashboardNameMissing,
            ));
        }
        if dashboard.path.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::DashboardPathMissing,
            ));
        }
    }
    Ok(())
}

fn validate_queue_bindings(bindings: &QueueBindings) -> std::result::Result<(), ManifestError> {
    for (binding, _) in bindings.iter_with_access() {
        if binding.queue_id.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::QueueIdMissing,
            ));
        }
        if binding.binding.trim().is_empty() {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::QueueBindingMissing,
            ));
        }
    }
    Ok(())
}

fn validate_object_bindings(
    capability: crate::Capability,
    bindings: &[ObjectBinding],
    _table: &str,
) -> std::result::Result<(), ManifestError> {
    for binding in bindings {
        if binding
            .object_id(capability)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .is_none()
        {
            return Err(ManifestError::Validation(
                ManifestValidationIssue::ObjectIdMissing,
            ));
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
        assert_eq!(parsed.project_name(), "peekaboo");
        assert_eq!(parsed.domain(), "leostera.dev");
        assert!(parsed.workspace.members.is_empty());
    }

    #[test]
    fn workspace_manifest_supports_project_metadata() {
        let raw = r#"[project]
name = "visit-counter"
domain = "visitcounter.dev"
server = "isolated"

[workspace]
members = ["api", "worker"]
"#;
        let parsed: WorkspaceManifest = toml::from_str(raw).unwrap();
        assert_eq!(parsed.project_name(), "visit-counter");
        assert_eq!(parsed.domain(), "visitcounter.dev");
        assert_eq!(parsed.server(), Some("isolated"));
        assert_eq!(parsed.members(), &["api".to_owned(), "worker".to_owned()]);
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
        assert!(matches!(
            validate_str(raw, "gumgum.toml").unwrap_err(),
            ManifestError::Parse { .. }
        ));
    }

    #[test]
    fn secret_observability_and_dashboards_parse_and_validate() {
        let raw = r#"
[project]
namespace = "visit-counter"

[worker]
name = "api"

[observability]
enable = true
prometheus_metrics = "/_/metrics"

[observability.grafana]
sources = "./grafana/sources.json"

[[secret]]
name = "kava.fund/path/to/secret"
binding = "ENV_VAR_FOR_SECRET"

[[dashboard]]
name = "Request Latency"
path = "./grafana/request-latency.json"
"#;
        let report = validate_str(raw, "gumgum.toml").expect("manifest validates");
        assert!(report.ok);
        let manifest: WorkerManifest = toml::from_str(raw).expect("manifest parses");
        assert_eq!(manifest.secrets[0].binding, "ENV_VAR_FOR_SECRET");
        let observability = manifest.observability.unwrap();
        assert!(observability.enable);
        assert_eq!(observability.prometheus_metrics, "/_/metrics");
        assert_eq!(
            observability.grafana.unwrap().sources.as_deref(),
            Some("./grafana/sources.json")
        );
        assert_eq!(manifest.dashboards[0].name, "Request Latency");
    }

    #[test]
    fn observability_metrics_path_defaults_when_missing() {
        let raw = r#"
[worker]
name = "api"

[observability]
enable = true
"#;
        let report = validate_str(raw, "gumgum.toml").expect("manifest validates");
        assert!(report.ok);
        let manifest: WorkerManifest = toml::from_str(raw).expect("manifest parses");
        assert_eq!(
            manifest.observability.unwrap().prometheus_metrics,
            "/_/metrics"
        );
    }

    #[test]
    fn observability_metrics_path_must_be_absolute() {
        let raw = r#"
[worker]
name = "api"

[observability]
enable = true
prometheus_metrics = "metrics"
"#;
        assert!(matches!(
            validate_str(raw, "gumgum.toml").unwrap_err(),
            ManifestError::Validation(ManifestValidationIssue::PrometheusMetricsPathRelative)
        ));
    }

    #[test]
    fn observability_metrics_path_must_not_be_empty() {
        let raw = r#"
[worker]
name = "api"

[observability]
enable = true
prometheus_metrics = ""
"#;
        assert!(matches!(
            validate_str(raw, "gumgum.toml").unwrap_err(),
            ManifestError::Validation(ManifestValidationIssue::PrometheusMetricsPathRelative)
        ));
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
        assert!(matches!(
            validate_str(raw, "gumgum.toml").unwrap_err(),
            ManifestError::Parse { .. }
        ));
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
        assert!(matches!(
            validate_str(raw, "gumgum.toml").unwrap_err(),
            ManifestError::Parse { .. }
        ));
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
        assert!(workspace.manifest.contains("[project]"));
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
