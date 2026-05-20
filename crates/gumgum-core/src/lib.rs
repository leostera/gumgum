pub mod container_reconciler;
pub mod graph_store;
pub mod manifest;

pub use container_reconciler::{ContainerReconciler, DeployRequest};
pub use graph_store::{
    DesiredDeploy, GlobalObject, GraphStore, WorkerBinding, connection_examples, object_dns,
    provider_for_object,
};
pub use manifest::{
    Ingress, Limits, ManifestKind, ObjectBinding, Observability, Project, ValidationReport, Worker,
    WorkerManifest, Workspace, WorkspaceManifest, Zone, load_worker_path, load_workspace_path,
    validate_path, validate_str,
};

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, GumgumError>;

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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
}

impl GraphNode {
    pub fn new(id: impl Into<String>, kind: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

impl GraphEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub action: String,
}

impl PlanNode {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        label: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            action: action.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlanEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

impl PlanEdge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PlanGraph {
    pub nodes: Vec<PlanNode>,
    pub edges: Vec<PlanEdge>,
    pub execution_levels: Vec<Vec<String>>,
}

impl PlanGraph {
    pub fn new(nodes: Vec<PlanNode>, edges: Vec<PlanEdge>) -> Self {
        let execution_levels = topological_levels(
            nodes.iter().map(|node| node.id.clone()),
            edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone())),
        );
        Self {
            nodes,
            edges,
            execution_levels,
        }
    }
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_node(mut self, node: GraphNode) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn with_edge(mut self, edge: GraphEdge) -> Self {
        self.edges.push(edge);
        self
    }

    pub fn topological_levels(&self) -> Vec<Vec<String>> {
        topological_levels(
            self.nodes.iter().map(|node| node.id.clone()),
            self.edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone())),
        )
    }
}

fn topological_levels(
    nodes: impl IntoIterator<Item = String>,
    edges: impl IntoIterator<Item = (String, String)>,
) -> Vec<Vec<String>> {
    let edges = edges.into_iter().collect::<Vec<_>>();
    let mut remaining = nodes.into_iter().collect::<std::collections::BTreeSet<_>>();
    let mut levels = Vec::new();
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|id| {
                edges
                    .iter()
                    .all(|(from, to)| to != *id || !remaining.contains(from))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            levels.push(remaining.iter().cloned().collect());
            break;
        }
        for id in &ready {
            remaining.remove(id);
        }
        levels.push(ready);
    }
    levels
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Db,
    Kv,
    Blob,
    Queue,
    Observability,
    Manual,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Db => "db",
            Self::Kv => "kv",
            Self::Blob => "blob",
            Self::Queue => "queue",
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
