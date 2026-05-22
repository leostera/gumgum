pub mod cloudflare;
pub mod config_store;
pub mod container_reconciler;
pub mod daemon_health;
pub mod deployment;
pub mod docker_engine;
pub mod domain;
pub mod graph;
pub mod graph_store;
pub mod internal_db;
pub mod manifest;
pub mod platform;
pub mod process;
pub mod providers;
pub mod setup;
pub mod setup_installer;

pub use cloudflare::{CloudflareGrant, IngressMode};
pub use config_store::{ConfigScope, ConfigStore, ServerRecord};
pub use container_reconciler::{ContainerReconciler, DeployRequest};
pub use daemon_health::{DaemonHealthClient, DaemonPingReport};
pub use deployment::DeploymentDescriptor;
pub use docker_engine::{ContainerSnapshot, DockerEngine};
pub use domain::{DomainProvider, DomainRecord};
pub use graph::{
    BindingName, ContainerName, DesiredGraph, DesiredGraphNode, GraphActionExecutor,
    GraphActionPlanner, GraphExecutionContext, GraphExecutionStep, GraphExecutionTarget,
    GraphNodeId, GraphReconcileAction, GraphReconciler, GraphReconciliationPlan, HealthPath,
    ImageName, ObjectName, ObjectRef, Port, ProviderName, RouteHost, WorkerId,
};
pub use graph_store::{
    ControlPlaneEventKind, DeploymentRevision, DesiredDeploy, DesiredProvider, GlobalObject,
    GraphStore, NewReconcileEvent, ReconcileEvent, ReconcileEventId, ReconcileEventStatus,
    WorkerBinding, new_operation_id, object_dns, projected_binding_env,
};
pub use manifest::{
    Ingress, InitManifestKind, InitPlan, Limits, ManifestKind, ObjectBinding, Observability,
    Project, QueueBinding, QueueBindings, ScaffoldFile, ValidationReport, Worker, WorkerManifest,
    Workspace, WorkspaceManifest, WorkspaceProject, Zone, init_plan, load_worker_path,
    load_workspace_path, validate_path, validate_str, worker_manifest_template,
    worker_scaffold_files, workspace_manifest_template,
};
pub use platform::LocalPlatform;
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
                likely_cause,
                next_commands,
            } => ErrorReport {
                ok: false,
                subsystem: *subsystem,
                code: *code,
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
            likely_cause: self.likely_cause,
            next_commands: self.next_commands,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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
pub struct ErrorReport {
    pub ok: bool,
    pub subsystem: Subsystem,
    pub code: ErrorCode,
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
    pub message: String,
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
    pub message: String,
}

pub mod presentation_graph;
pub use presentation_graph::{
    Graph, GraphEdge, GraphNode, PresentationGraph, PresentationGraphEdge, PresentationGraphNode,
    affected_subgraph, render_mermaid_graph,
};

pub mod plan_graph;
pub use plan_graph::{PlanEdge, PlanGraph, PlanNode};

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
            Self::Secret => "onepassword.main",
            Self::Observability => "otel.platform",
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

    pub fn plan_lines(&self) -> Vec<String> {
        self.graph()
            .execution_levels
            .iter()
            .enumerate()
            .flat_map(|(index, level)| {
                level
                    .iter()
                    .map(move |node| format!("level {}: {node}", index + 1))
            })
            .collect()
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
                    "collect manifest desired state",
                ),
                PlanNode::new(
                    "actual/containers",
                    "source",
                    "docker state",
                    "collect actual container state",
                ),
                PlanNode::new(
                    "provider/registry.platform",
                    "provider",
                    "registry.platform",
                    "ensure local registry provider is running",
                ),
                PlanNode::new(
                    format!("image/{worker}"),
                    "image",
                    worker,
                    "build and push worker image",
                ),
                PlanNode::new(
                    format!("container/{worker}"),
                    "container",
                    worker,
                    "reconcile worker container",
                ),
                PlanNode::new(
                    format!("health/{worker}"),
                    "health_check",
                    worker,
                    "verify health check and routes",
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
            "ensure provider is running",
        ));
        self.nodes.push(PlanNode::new(
            &object_id,
            "global_object",
            object,
            "ensure global object exists",
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
                "ensure worker-local binding exists",
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
            observability: None,
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
