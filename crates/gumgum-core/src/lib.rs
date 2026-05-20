use serde::{Deserialize, Serialize};
use std::fmt;
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
        let mut remaining = self
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut levels = Vec::new();
        while !remaining.is_empty() {
            let ready = remaining
                .iter()
                .filter(|id| {
                    !self
                        .edges
                        .iter()
                        .any(|edge| edge.to == **id && remaining.contains(&edge.from))
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
}
