pub mod actions;
pub mod cloudflare;
pub mod config_store;
pub mod container_reconciler;
pub mod daemon_health;
pub mod deployment;
pub mod docker_engine;
pub mod domain;
pub mod events;
pub mod graph;
pub mod graph_store;
pub mod internal_db;
pub mod manifest;
pub mod platform;
pub mod process;
pub mod providers;
pub mod setup;
pub mod setup_installer;

pub use actions::{ActionScope, ConnectionExample, CoreAction, CoreActions, SetupStep};
pub use cloudflare::{CloudflareGrant, IngressMode};
pub use config_store::{ConfigScope, ConfigStore, ServerRecord};
pub use container_reconciler::{ContainerReconciler, DeployRequest};
pub use daemon_health::{DaemonHealthClient, DaemonPingReport};
pub use deployment::DeploymentDescriptor;
pub use docker_engine::{ContainerRunSpec, ContainerSnapshot, DockerEngine, PortBindingSpec};
pub use domain::{DomainProvider, DomainRecord};
pub use events::GumgumEvent;
pub use graph::{
    ActionGraph, BindingName, ContainerName, CurrentGraph, DesiredGraph, DesiredGraphNode,
    GraphActionExecutor, GraphActionPlanner, GraphExecutionContext, GraphExecutionReport,
    GraphExecutionStep, GraphExecutionTarget, GraphMutation, GraphNodeId, GraphReconcileAction,
    GraphReconciler, GraphReconciliationPlan, GumgumAction, HealthPath, ImageName, ObjectName,
    ObjectRef, Port, ProviderName, RouteHost, WorkerId,
};
pub use graph_store::{
    ControlPlaneEventKind, DeploymentRevision, DesiredDeploy, DesiredProvider, GlobalObject,
    GraphStore, GraphTransitionPreview, NewReconcileEvent, ReconcileEvent, ReconcileEventId,
    ReconcileEventStatus, WorkerBinding, new_operation_id, object_dns, projected_binding_env,
};
pub use manifest::{
    Dashboard, GrafanaObservability, Ingress, InitManifestKind, InitPlan, Limits, ManifestKind,
    ObjectBinding, Observability, Project, QueueBinding, QueueBindings, ScaffoldFile,
    SecretBinding, ValidationReport, Worker, WorkerManifest, Workspace, WorkspaceManifest,
    WorkspaceProject, Zone, init_plan, load_worker_path, load_workspace_path, validate_path,
    validate_str, worker_manifest_template, worker_scaffold_files, workspace_manifest_template,
};
pub use platform::{LocalPlatform, PlatformEvent, PlatformStep};
pub use process::{run_setup_command, run_setup_command_streaming};
pub use providers::{
    ObjectProviderPlan, ProviderConfig, ProviderCredentials, ProviderReconciler, ProviderSpec,
    ProviderStatus, connection_examples, generated_secret_value, object_provider_plan,
    provider_spec,
};
pub use setup::{not_configured_status, setup_actions};
pub use setup_installer::{GumgumInstaller, SetupOptions, SetupTarget};

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, GumgumError>;

pub fn derive_test_domain(root_domain: &str) -> String {
    let root = root_domain.trim_end_matches('.');
    match root.rsplit_once('.') {
        Some((name, _)) => format!("{name}.test"),
        None => format!("{root}.test"),
    }
}

pub fn default_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .map(|name| sanitize_name(&name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "hello".to_owned())
}

pub fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[derive(Debug, Error)]
pub enum GumgumError {
    #[error("{message}")]
    Structured {
        subsystem: Subsystem,
        code: ErrorCode,
        message: String,
        kind: Option<ErrorKind>,
        likely_cause: Option<String>,
        next_commands: Vec<String>,
    },
}

impl GumgumError {
    pub fn structured(
        subsystem: Subsystem,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> ErrorBuilder {
        ErrorBuilder {
            subsystem,
            code,
            message: message.into(),
            kind: None,
            likely_cause: None,
            next_commands: Vec::new(),
        }
    }

    pub fn structured_kind(subsystem: Subsystem, code: ErrorCode, kind: ErrorKind) -> ErrorBuilder {
        ErrorBuilder {
            subsystem,
            code,
            message: kind.machine_code().to_owned(),
            kind: Some(kind),
            likely_cause: None,
            next_commands: Vec::new(),
        }
    }

    pub fn to_report(&self) -> ErrorReport {
        match self {
            GumgumError::Structured {
                subsystem,
                code,
                message,
                kind,
                likely_cause,
                next_commands,
            } => ErrorReport {
                ok: false,
                error: ErrorDescriptor {
                    subsystem: *subsystem,
                    code: *code,
                },
                subsystem: *subsystem,
                code: *code,
                kind: *kind,
                message: message.clone(),
                likely_cause: likely_cause.clone(),
                next_commands: next_commands.clone(),
            },
        }
    }
}

#[derive(Debug)]
pub struct ErrorBuilder {
    subsystem: Subsystem,
    code: ErrorCode,
    message: String,
    kind: Option<ErrorKind>,
    likely_cause: Option<String>,
    next_commands: Vec<String>,
}

impl ErrorBuilder {
    pub fn likely_cause(mut self, value: impl Into<String>) -> Self {
        self.likely_cause = Some(value.into());
        self
    }

    pub fn next_command(mut self, value: impl Into<String>) -> Self {
        self.next_commands.push(value.into());
        self
    }

    pub fn build(self) -> GumgumError {
        GumgumError::Structured {
            subsystem: self.subsystem,
            code: self.code,
            message: self.message,
            kind: self.kind,
            likely_cause: self.likely_cause,
            next_commands: self.next_commands,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Subsystem {
    Cli,
    Manifest,
    Schema,
    Config,
    Api,
    Doctor,
    Setup,
}

impl fmt::Display for Subsystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Subsystem::Cli => "cli",
            Subsystem::Manifest => "manifest",
            Subsystem::Schema => "schema",
            Subsystem::Config => "config",
            Subsystem::Api => "api",
            Subsystem::Doctor => "doctor",
            Subsystem::Setup => "setup",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgs,
    Io,
    ManifestNotFound,
    ManifestParseFailed,
    ManifestValidationFailed,
    NotImplemented,
}

#[derive(Debug, Serialize)]
pub struct ErrorDescriptor {
    pub subsystem: Subsystem,
    pub code: ErrorCode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    HomeReadFailed,
    ConfigDirectoryCreateFailed,
    ConfigReadFailed,
    ConfigParseFailed,
    ConfigWriteFailed,
    DomainListReadFailed,
    DomainListParseFailed,
    DomainListWriteFailed,
    ServerListReadFailed,
    ServerListParseFailed,
    ServerListWriteFailed,
    CloudflareGrantReadFailed,
    CloudflareGrantParseFailed,
    CloudflareGrantWriteFailed,
    ProviderConfigReadFailed,
    ProviderConfigParseFailed,
    ProviderConfigWriteFailed,
    ProviderCredentialsReadFailed,
    ProviderCredentialsParseFailed,
    ProviderCredentialsWriteFailed,
    GraphDirectoryCreateFailed,
    GraphDatabaseUrlBuildFailed,
    GraphDatabaseOpenFailed,
    GraphDatabaseMigrationFailed,
    SetupCommandSpawnFailed,
    SetupCommandFailed,
    GraphValueInvalid,
    ControlPlaneEventKindUnknown,
    ReconcileEventStatusUnknown,
    ManifestReadFailed,
    ManifestParseFailed,
    ManifestValidationFailed,
    HttpClientBuildFailed,
    DaemonReachFailed,
    DaemonReturnedError,
    DaemonInvalidJson,
    DockerDaemonRequestFailed,
    DockerExecFailed,
    DnsmasqConfigWriteFailed,
    DnsmasqConfigDirectoryCreateFailed,
    DeploymentContainerHealthCheckFailed,
    GraphExecutionInjectedFailure,
    CloudflareZoneNotFound,
    CloudflareTunnelCreateResponseDecodeFailed,
    CloudflareTunnelTokenResponseDecodeFailed,
    CloudflareApiRequestFailed,
    CloudflareApiReturnedError,
    CloudflareApiResponseBodyReadFailed,
    CloudflareApiResponseDecodeFailed,
    CloudflareApiResultMissing,
    CloudflareTokenRequired,
    CloudflareTokenEmpty,
    PublishedRouteDomainNotManaged,
    ProviderCredentialsMissing,
    PostgresProviderReadinessFailed,
    PostgresDatabaseCreateFailed,
    MinioObjectInvalidUtf8,
    MinioProviderContainerInspectFailed,
    MinioProviderContainerNetworkAddressMissing,
    MinioS3ApiRequestFailed,
    MinioS3ListResponseReadFailed,
    MinioBucketObjectReadFailed,
    MinioS3ApiReturnedError,
    BucketObjectPathInvalid,
    PrometheusScrapeStateReadFailed,
    PrometheusScrapeStateParseFailed,
    PrometheusScrapeStateSerializeFailed,
    PrometheusStateDirectoryCreateFailed,
    PrometheusScrapeStateWriteFailed,
    AlloyConfigDirectoryCreateFailed,
    AlloyConfigWriteFailed,
    OTelConfigDirectoryCreateFailed,
    OTelConfigWriteFailed,
    TempoConfigDirectoryCreateFailed,
    TempoConfigWriteFailed,
    PrometheusConfigDirectoryCreateFailed,
    PrometheusConfigWriteFailed,
    GrafanaContainerNotRunning,
    GrafanaContainerNetworkMissing,
    GrafanaDatasourceArtifactInvalid,
    GrafanaArtifactKindUnsupported,
    GrafanaDatasourceUidMissing,
    GrafanaApiRequestFailed,
    GrafanaApiReturnedError,
    SetupBinaryLocateFailed,
    SetupDaemonDirectoryCreateFailed,
    SetupBinDirectoryCreateFailed,
    SetupLocalDaemonInstallFailed,
    SetupUserSystemdDirectoryCreateFailed,
    SetupLocalUserServiceWriteFailed,
    SetupLocalHostnameReadFailed,
    SetupRemoteHostnameReadFailed,
    SetupRemoteHostnameCommandFailed,
}

impl ErrorKind {
    pub fn machine_code(self) -> &'static str {
        match self {
            ErrorKind::HomeReadFailed => "config.home.read_failed",
            ErrorKind::ConfigDirectoryCreateFailed => "config.directory.create_failed",
            ErrorKind::ConfigReadFailed => "config.read_failed",
            ErrorKind::ConfigParseFailed => "config.parse_failed",
            ErrorKind::ConfigWriteFailed => "config.write_failed",
            ErrorKind::DomainListReadFailed => "config.domain_list.read_failed",
            ErrorKind::DomainListParseFailed => "config.domain_list.parse_failed",
            ErrorKind::DomainListWriteFailed => "config.domain_list.write_failed",
            ErrorKind::ServerListReadFailed => "config.server_list.read_failed",
            ErrorKind::ServerListParseFailed => "config.server_list.parse_failed",
            ErrorKind::ServerListWriteFailed => "config.server_list.write_failed",
            ErrorKind::CloudflareGrantReadFailed => "config.cloudflare_grant.read_failed",
            ErrorKind::CloudflareGrantParseFailed => "config.cloudflare_grant.parse_failed",
            ErrorKind::CloudflareGrantWriteFailed => "config.cloudflare_grant.write_failed",
            ErrorKind::ProviderConfigReadFailed => "config.provider_config.read_failed",
            ErrorKind::ProviderConfigParseFailed => "config.provider_config.parse_failed",
            ErrorKind::ProviderConfigWriteFailed => "config.provider_config.write_failed",
            ErrorKind::ProviderCredentialsReadFailed => "config.provider_credentials.read_failed",
            ErrorKind::ProviderCredentialsParseFailed => "config.provider_credentials.parse_failed",
            ErrorKind::ProviderCredentialsWriteFailed => "config.provider_credentials.write_failed",
            ErrorKind::GraphDirectoryCreateFailed => "config.graph_directory.create_failed",
            ErrorKind::GraphDatabaseUrlBuildFailed => "config.graph_database.url_build_failed",
            ErrorKind::GraphDatabaseOpenFailed => "config.graph_database.open_failed",
            ErrorKind::GraphDatabaseMigrationFailed => "config.graph_database.migration_failed",
            ErrorKind::SetupCommandSpawnFailed => "setup.command.spawn_failed",
            ErrorKind::SetupCommandFailed => "setup.command.failed",
            ErrorKind::GraphValueInvalid => "graph.value.invalid",
            ErrorKind::ControlPlaneEventKindUnknown => "graph.control_plane_event_kind.unknown",
            ErrorKind::ReconcileEventStatusUnknown => "graph.reconcile_event_status.unknown",
            ErrorKind::ManifestReadFailed => "manifest.read_failed",
            ErrorKind::ManifestParseFailed => "manifest.parse_failed",
            ErrorKind::ManifestValidationFailed => "manifest.validation_failed",
            ErrorKind::HttpClientBuildFailed => "api.http_client.build_failed",
            ErrorKind::DaemonReachFailed => "api.daemon.reach_failed",
            ErrorKind::DaemonReturnedError => "api.daemon.returned_error",
            ErrorKind::DaemonInvalidJson => "api.daemon.invalid_json",
            ErrorKind::DockerDaemonRequestFailed => "setup.docker.request_failed",
            ErrorKind::DockerExecFailed => "setup.docker.exec_failed",
            ErrorKind::DnsmasqConfigWriteFailed => "setup.dnsmasq_config.write_failed",
            ErrorKind::DnsmasqConfigDirectoryCreateFailed => {
                "setup.dnsmasq_config_directory.create_failed"
            }
            ErrorKind::DeploymentContainerHealthCheckFailed => {
                "api.deployment_container.health_check_failed"
            }
            ErrorKind::GraphExecutionInjectedFailure => "setup.graph_execution.injected_failure",
            ErrorKind::CloudflareZoneNotFound => "cloudflare.zone.not_found",
            ErrorKind::CloudflareTunnelCreateResponseDecodeFailed => {
                "cloudflare.tunnel_create_response.decode_failed"
            }
            ErrorKind::CloudflareTunnelTokenResponseDecodeFailed => {
                "cloudflare.tunnel_token_response.decode_failed"
            }
            ErrorKind::CloudflareApiRequestFailed => "cloudflare.api.request_failed",
            ErrorKind::CloudflareApiReturnedError => "cloudflare.api.returned_error",
            ErrorKind::CloudflareApiResponseBodyReadFailed => {
                "cloudflare.api.response_body_read_failed"
            }
            ErrorKind::CloudflareApiResponseDecodeFailed => "cloudflare.api.response_decode_failed",
            ErrorKind::CloudflareApiResultMissing => "cloudflare.api.result_missing",
            ErrorKind::CloudflareTokenRequired => "cloudflare.token.required",
            ErrorKind::CloudflareTokenEmpty => "cloudflare.token.empty",
            ErrorKind::PublishedRouteDomainNotManaged => {
                "cloudflare.published_route_domain.not_managed"
            }
            ErrorKind::ProviderCredentialsMissing => "provider.credentials.missing",
            ErrorKind::PostgresProviderReadinessFailed => "provider.postgres.readiness_failed",
            ErrorKind::PostgresDatabaseCreateFailed => "provider.postgres.database_create_failed",
            ErrorKind::MinioObjectInvalidUtf8 => "provider.minio.object.invalid_utf8",
            ErrorKind::MinioProviderContainerInspectFailed => {
                "provider.minio.container.inspect_failed"
            }
            ErrorKind::MinioProviderContainerNetworkAddressMissing => {
                "provider.minio.container.network_address_missing"
            }
            ErrorKind::MinioS3ApiRequestFailed => "provider.minio.s3_api.request_failed",
            ErrorKind::MinioS3ListResponseReadFailed => {
                "provider.minio.s3_api.list_response_read_failed"
            }
            ErrorKind::MinioBucketObjectReadFailed => "provider.minio.bucket_object.read_failed",
            ErrorKind::MinioS3ApiReturnedError => "provider.minio.s3_api.returned_error",
            ErrorKind::BucketObjectPathInvalid => "provider.bucket_object_path.invalid",
            ErrorKind::PrometheusScrapeStateReadFailed => {
                "observability.prometheus_scrape_state.read_failed"
            }
            ErrorKind::PrometheusScrapeStateParseFailed => {
                "observability.prometheus_scrape_state.parse_failed"
            }
            ErrorKind::PrometheusScrapeStateSerializeFailed => {
                "observability.prometheus_scrape_state.serialize_failed"
            }
            ErrorKind::PrometheusStateDirectoryCreateFailed => {
                "observability.prometheus_state_directory.create_failed"
            }
            ErrorKind::PrometheusScrapeStateWriteFailed => {
                "observability.prometheus_scrape_state.write_failed"
            }
            ErrorKind::AlloyConfigDirectoryCreateFailed => {
                "observability.alloy_config_directory.create_failed"
            }
            ErrorKind::AlloyConfigWriteFailed => "observability.alloy_config.write_failed",
            ErrorKind::OTelConfigDirectoryCreateFailed => {
                "observability.otel_config_directory.create_failed"
            }
            ErrorKind::OTelConfigWriteFailed => "observability.otel_config.write_failed",
            ErrorKind::TempoConfigDirectoryCreateFailed => {
                "observability.tempo_config_directory.create_failed"
            }
            ErrorKind::TempoConfigWriteFailed => "observability.tempo_config.write_failed",
            ErrorKind::PrometheusConfigDirectoryCreateFailed => {
                "observability.prometheus_config_directory.create_failed"
            }
            ErrorKind::PrometheusConfigWriteFailed => {
                "observability.prometheus_config.write_failed"
            }
            ErrorKind::GrafanaContainerNotRunning => "observability.grafana.container_not_running",
            ErrorKind::GrafanaContainerNetworkMissing => {
                "observability.grafana.container_network_missing"
            }
            ErrorKind::GrafanaDatasourceArtifactInvalid => {
                "observability.grafana.datasource_artifact_invalid"
            }
            ErrorKind::GrafanaArtifactKindUnsupported => {
                "observability.grafana.artifact_kind_unsupported"
            }
            ErrorKind::GrafanaDatasourceUidMissing => {
                "observability.grafana.datasource_uid_missing"
            }
            ErrorKind::GrafanaApiRequestFailed => "observability.grafana.api_request_failed",
            ErrorKind::GrafanaApiReturnedError => "observability.grafana.api_returned_error",
            ErrorKind::SetupBinaryLocateFailed => "setup.binary.locate_failed",
            ErrorKind::SetupDaemonDirectoryCreateFailed => "setup.daemon_directory.create_failed",
            ErrorKind::SetupBinDirectoryCreateFailed => "setup.bin_directory.create_failed",
            ErrorKind::SetupLocalDaemonInstallFailed => "setup.local_daemon.install_failed",
            ErrorKind::SetupUserSystemdDirectoryCreateFailed => {
                "setup.user_systemd_directory.create_failed"
            }
            ErrorKind::SetupLocalUserServiceWriteFailed => "setup.local_user_service.write_failed",
            ErrorKind::SetupLocalHostnameReadFailed => "setup.local_hostname.read_failed",
            ErrorKind::SetupRemoteHostnameReadFailed => "setup.remote_hostname.read_failed",
            ErrorKind::SetupRemoteHostnameCommandFailed => "setup.remote_hostname.command_failed",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorReport {
    pub ok: bool,
    pub error: ErrorDescriptor,
    pub subsystem: Subsystem,
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ErrorKind>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likely_cause: Option<String>,
    pub next_commands: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub ok: bool,
    pub configured: bool,
    pub daemon: DaemonStatus,
    pub status: StatusMessage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusMessage {
    NotConfigured,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonStatus {
    NotConfigured,
    Unknown,
    Healthy,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub status: DoctorCheckStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    Passed,
    Failed,
}

pub mod presentation_graph;
pub use presentation_graph::{
    Graph, GraphEdge, GraphNode, PresentationGraph, PresentationGraphEdge, PresentationGraphNode,
    affected_subgraph, render_mermaid_graph,
};

pub mod plan_graph;
pub use plan_graph::{PlanAction, PlanEdge, PlanGraph, PlanNode};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Db,
    Kv,
    Blob,
    Queue,
    Secret,
    Observability,
    Manual,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Db => "db",
            Self::Kv => "kv",
            Self::Blob => "bucket",
            Self::Queue => "queue",
            Self::Secret => "secret",
            Self::Observability => "observability",
            Self::Manual => "manual",
        }
    }

    pub fn provider(self) -> &'static str {
        match self {
            Self::Db => "postgres.main",
            Self::Kv => "redis.main",
            Self::Blob => "minio.main",
            Self::Queue => "redpanda.main",
            Self::Secret => "secrets.platform",
            Self::Observability => "observability.platform",
            Self::Manual => "manual.main",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Capability {
    type Err = ();

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match value {
            "db" | "database" => Self::Db,
            "kv" => Self::Kv,
            "bucket" | "blob" => Self::Blob,
            "queue" => Self::Queue,
            "secret" | "secrets" => Self::Secret,
            "telemetry" | "observability" => Self::Observability,
            _ => Self::Manual,
        })
    }
}

#[derive(Clone, Debug)]
pub struct WorkerPlanInput {
    pub worker_name: String,
    pub databases: Vec<BindingPlanInput>,
    pub kvs: Vec<BindingPlanInput>,
    pub buckets: Vec<BindingPlanInput>,
    pub queues: Vec<BindingPlanInput>,
}

#[derive(Clone, Debug)]
pub struct BindingPlanInput {
    pub capability: Capability,
    pub name: String,
    pub binding: Option<String>,
}

pub struct DeployPlanner {
    input: WorkerPlanInput,
}

impl DeployPlanner {
    pub fn new(input: WorkerPlanInput) -> Self {
        Self { input }
    }

    pub fn from_manifest(manifest: &WorkerManifest) -> Self {
        Self::new(WorkerPlanInput {
            worker_name: manifest.worker.name.clone(),
            databases: manifest
                .database
                .iter()
                .map(|binding| BindingPlanInput {
                    capability: Capability::Db,
                    name: binding
                        .object_id(Capability::Db)
                        .unwrap_or_default()
                        .to_owned(),
                    binding: binding.binding.clone(),
                })
                .collect(),
            kvs: manifest
                .kv
                .iter()
                .map(|binding| BindingPlanInput {
                    capability: Capability::Kv,
                    name: binding
                        .object_id(Capability::Kv)
                        .unwrap_or_default()
                        .to_owned(),
                    binding: binding.binding.clone(),
                })
                .collect(),
            buckets: manifest
                .bucket
                .iter()
                .map(|binding| BindingPlanInput {
                    capability: Capability::Blob,
                    name: binding
                        .object_id(Capability::Blob)
                        .unwrap_or_default()
                        .to_owned(),
                    binding: binding.binding.clone(),
                })
                .collect(),
            queues: manifest
                .queue
                .iter_with_access()
                .map(|(binding, _access)| BindingPlanInput {
                    capability: Capability::Queue,
                    name: binding.queue_id.clone(),
                    binding: Some(binding.binding.clone()),
                })
                .collect(),
        })
    }

    pub fn graph(&self) -> PlanGraph {
        let worker = &self.input.worker_name;
        let mut graph = MutablePlanGraph::new(worker);
        for db in &self.input.databases {
            graph.add_binding(
                worker,
                db.capability.as_str(),
                &db.name,
                db.binding.as_deref(),
            );
        }
        for kv in &self.input.kvs {
            graph.add_binding(
                worker,
                kv.capability.as_str(),
                &kv.name,
                kv.binding.as_deref(),
            );
        }
        for bucket in &self.input.buckets {
            graph.add_binding(
                worker,
                bucket.capability.as_str(),
                &bucket.name,
                bucket.binding.as_deref(),
            );
        }
        for queue in &self.input.queues {
            graph.add_binding(
                worker,
                queue.capability.as_str(),
                &queue.name,
                queue.binding.as_deref(),
            );
        }
        graph.finish()
    }
}

struct MutablePlanGraph {
    nodes: Vec<PlanNode>,
    edges: Vec<PlanEdge>,
}

impl MutablePlanGraph {
    fn new(worker: &str) -> Self {
        Self {
            nodes: vec![
                PlanNode::new(
                    "source/manifests",
                    "source",
                    "gumgum.toml files",
                    PlanAction::CollectManifestDesiredState,
                ),
                PlanNode::new(
                    "actual/containers",
                    "source",
                    "docker state",
                    PlanAction::CollectActualContainerState,
                ),
                PlanNode::new(
                    "provider/registry.platform",
                    "provider",
                    "registry.platform",
                    PlanAction::EnsureLocalRegistryProvider,
                ),
                PlanNode::new(
                    format!("image/{worker}"),
                    "image",
                    worker,
                    PlanAction::BuildAndPushWorkerImage,
                ),
                PlanNode::new(
                    format!("container/{worker}"),
                    "container",
                    worker,
                    PlanAction::ReconcileWorkerContainer,
                ),
                PlanNode::new(
                    format!("health/{worker}"),
                    "health_check",
                    worker,
                    PlanAction::VerifyHealthCheckAndRoutes,
                ),
            ],
            edges: vec![
                PlanEdge::new(
                    "source/manifests",
                    format!("image/{worker}"),
                    "desired_state",
                ),
                PlanEdge::new(
                    "actual/containers",
                    format!("container/{worker}"),
                    "actual_state",
                ),
                PlanEdge::new(
                    "provider/registry.platform",
                    format!("image/{worker}"),
                    "backs",
                ),
                PlanEdge::new(
                    format!("image/{worker}"),
                    format!("container/{worker}"),
                    "created_from",
                ),
                PlanEdge::new(
                    format!("container/{worker}"),
                    format!("health/{worker}"),
                    "has_health_check",
                ),
            ],
        }
    }

    fn add_binding(&mut self, worker: &str, kind: &str, object: &str, binding: Option<&str>) {
        let capability = Capability::from_str(kind).unwrap_or(Capability::Manual);
        let provider = capability.provider();
        let object_id = format!("{capability}/{object}");
        self.nodes.push(PlanNode::new(
            format!("provider/{provider}"),
            "provider",
            provider,
            PlanAction::EnsureProviderRunning,
        ));
        self.nodes.push(PlanNode::new(
            &object_id,
            "global_object",
            object,
            PlanAction::EnsureGlobalObjectExists,
        ));
        self.edges.push(PlanEdge::new(
            "source/manifests",
            &object_id,
            "desired_state",
        ));
        self.edges.push(PlanEdge::new(
            format!("provider/{provider}"),
            &object_id,
            "backs",
        ));
        if let Some(binding) = binding {
            let binding_id = format!("binding/{worker}/{binding}");
            self.nodes.push(PlanNode::new(
                &binding_id,
                "binding",
                binding,
                PlanAction::EnsureWorkerLocalBindingExists,
            ));
            self.edges
                .push(PlanEdge::new(&object_id, &binding_id, "projects_as"));
            self.edges.push(PlanEdge::new(
                &binding_id,
                format!("container/{worker}"),
                "injects_into",
            ));
        }
    }

    fn finish(self) -> PlanGraph {
        PlanGraph::new(self.nodes, self.edges)
    }
}

#[cfg(test)]
mod deploy_planner_tests {
    use super::*;

    #[test]
    fn deploy_plan_includes_bucket_and_queue_bindings() {
        let manifest = WorkerManifest {
            project: None,
            worker: Worker {
                name: "visit-counter-api".to_owned(),
                image: Some("./Dockerfile".to_owned()),
                build_context: Some(".".to_owned()),
                command: None,
                port: Some(3000),
                checks: Default::default(),
                health: Some("/healthz".to_owned()),
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: Vec::new(),
            kv: Vec::new(),
            bucket: vec![ObjectBinding {
                bucket_id: Some("visit-requests".to_owned()),
                binding: Some("VISIT_REQUESTS_BUCKET".to_owned()),
                access: Some("read-write".to_owned()),
                ..Default::default()
            }],
            queue: QueueBindings {
                producer: vec![QueueBinding {
                    queue_id: "visit-events".to_owned(),
                    binding: "VISIT_EVENTS_QUEUE".to_owned(),
                }],
                consumer: Vec::new(),
            },
            secrets: Vec::new(),
            observability: None,
            dashboards: Vec::new(),
            limits: None,
        };

        let graph = DeployPlanner::from_manifest(&manifest).graph();
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "bucket/visit-requests")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "queue/visit-events")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "binding/visit-counter-api/VISIT_REQUESTS_BUCKET")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id == "binding/visit-counter-api/VISIT_EVENTS_QUEUE")
        );
    }
}

#[cfg(test)]
mod presentation_boundary_tests {
    use std::path::Path;

    #[test]
    fn core_sources_do_not_print_directly() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [
            concat!("print", "ln!"),
            concat!("eprint", "ln!"),
            concat!("eprint", "!"),
        ];
        let mut violations = Vec::new();
        collect_print_violations(&src, &forbidden, &mut violations);
        assert!(
            violations.is_empty(),
            "gumgum-core must report typed data and let gumgum-cli present it; direct printing found:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn core_sources_do_not_expose_presentation_string_action_lists() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [
            concat!("actions", ": Vec<String>"),
            concat!("provider_actions", ": Vec<String>"),
            concat!("binding_actions", ": Vec<String>"),
            concat!("connection_examples", ": Vec<String>"),
            concat!("pub fn ", "plan_lines"),
            concat!("collect manifest", " desired state"),
            concat!("build and push", " worker image"),
            concat!("ensure provider", " is running"),
            concat!("verify health", " check and routes"),
            concat!("read deployed", " worker"),
            concat!("plan route", " mapping"),
            concat!("workspace manifest", " is valid"),
            concat!("worker manifest", " is valid"),
            concat!("project.name", " must"),
            concat!("worker.name", " must"),
            concat!("secret binding", " must"),
            concat!("dashboard", " must declare"),
            concat!("manifest validation", " failed"),
            concat!("ensure deploy runtime", " for"),
            concat!("ensure route", " points at"),
            concat!("ensure binding", " projects"),
            concat!("remove deployment", " "),
            concat!("remove graph", " node"),
        ];
        let mut violations = Vec::new();
        collect_print_violations(&src, &forbidden, &mut violations);
        assert!(
            violations.is_empty(),
            "gumgum-core must expose symbolic action/example data, not rendered user strings:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn core_sources_do_not_construct_prose_errors() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let forbidden = [concat!("GumgumError", "::structured(")];
        let mut violations = Vec::new();
        collect_print_violations(&src, &forbidden, &mut violations);
        assert!(
            violations.is_empty(),
            "gumgum-core must use symbolic ErrorKind values, not prose error constructors:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn core_status_reports_use_symbolic_status_not_rendered_messages() {
        let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs");
        let status_block = lib
            .split("pub struct StatusReport")
            .nth(1)
            .and_then(|tail| tail.split("#[derive").next())
            .expect("StatusReport block exists");
        assert!(status_block.contains("pub status: StatusMessage"));
        assert!(!status_block.contains("message: String"));

        let doctor_block = lib
            .split("pub struct DoctorCheck")
            .nth(1)
            .and_then(|tail| tail.split("#[derive").next())
            .expect("DoctorCheck block exists");
        assert!(doctor_block.contains("pub status: DoctorCheckStatus"));
        assert!(!doctor_block.contains("message: String"));
    }

    fn collect_print_violations(path: &Path, forbidden: &[&str], violations: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_print_violations(&path, forbidden, violations);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (index, line) in contents.lines().enumerate() {
                if forbidden.iter().any(|needle| line.contains(needle)) {
                    violations.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
                }
            }
        }
    }
}
