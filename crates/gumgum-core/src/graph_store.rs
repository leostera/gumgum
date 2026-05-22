use crate::{
    BindingName, Capability, ContainerName, DesiredGraph, DesiredGraphNode, ErrorCode,
    GraphActionPlanner, GraphEdge, GraphExecutionStep, GraphNode, GumgumError, HealthPath,
    ImageName, ObjectName, ObjectRef, Port, ProviderName, Result, RouteHost, Subsystem, WorkerId,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesiredDeploy {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: Option<String>,
    pub port: u16,
    pub health: String,
}

impl DesiredDeploy {
    pub fn graph_node(&self) -> Result<DesiredGraphNode> {
        Ok(DesiredGraphNode::Deployment {
            worker: WorkerId::new(&self.worker)?,
            image: ImageName::new(&self.image)?,
            container: ContainerName::new(&self.container)?,
            route: self.route.as_deref().map(RouteHost::new).transpose()?,
            port: Port::new(self.port)?,
            health: HealthPath::new(&self.health)?,
        })
    }

    pub fn execution_step(&self) -> GraphExecutionStep {
        GraphActionPlanner::ensure_deploy_step(
            WorkerId::new(&self.worker).unwrap_or_else(|_| WorkerId::new("worker").unwrap()),
            ContainerName::new(&self.container)
                .unwrap_or_else(|_| ContainerName::new("container").unwrap()),
            ImageName::new(&self.image)
                .unwrap_or_else(|_| ImageName::new("invalid:latest").unwrap()),
            self.route
                .as_deref()
                .and_then(|route| RouteHost::new(route).ok()),
            Port::new(self.port).unwrap_or_else(|_| Port::new(80).unwrap()),
            HealthPath::new(&self.health).unwrap_or_else(|_| HealthPath::new("/healthz").unwrap()),
        )
    }

    pub async fn reconciliation_steps(&self, graph_path: PathBuf) -> Vec<GraphExecutionStep> {
        let deploy = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = GraphStore::new(graph_path);
            let old_graph = store.load_desired_graph()?;
            let mut new_graph = old_graph.clone();
            new_graph.nodes.insert(deploy.graph_node()?);
            Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    }

    pub async fn delete_reconciliation_steps(
        &self,
        graph_path: PathBuf,
    ) -> Vec<GraphExecutionStep> {
        let deploy = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = GraphStore::new(graph_path);
            let old_graph = store.load_desired_graph()?;
            let mut new_graph = old_graph.clone();
            new_graph.nodes.remove(&deploy.graph_node()?);
            Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconcileEventId(i64);

impl ReconcileEventId {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneEventKind {
    Mutation,
    Reconciliation,
}

impl ControlPlaneEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mutation => "mutation",
            Self::Reconciliation => "reconciliation",
        }
    }
}

impl std::fmt::Display for ControlPlaneEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ControlPlaneEventKind {
    type Err = GumgumError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "mutation" => Ok(Self::Mutation),
            "reconciliation" => Ok(Self::Reconciliation),
            _ => Err(GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "unknown control plane event kind",
            )
            .build()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileEventStatus {
    Planned,
    Executed,
    Failed,
}

impl ReconcileEventStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Executed => "executed",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for ReconcileEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReconcileEventStatus {
    type Err = GumgumError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "planned" => Ok(Self::Planned),
            "executed" => Ok(Self::Executed),
            "failed" => Ok(Self::Failed),
            _ => Err(GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "unknown reconciliation event status",
            )
            .build()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReconcileEvent {
    pub id: ReconcileEventId,
    pub kind: ControlPlaneEventKind,
    pub status: ReconcileEventStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub target: String,
    pub action: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NewReconcileEvent {
    pub kind: ControlPlaneEventKind,
    pub status: ReconcileEventStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub target: String,
    pub action: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeploymentRevision {
    pub id: i64,
    pub deploy: DesiredDeploy,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalObject {
    pub capability: Capability,
    pub name: String,
    pub namespace: String,
    pub root_domain: String,
}

impl GlobalObject {
    pub fn graph_node(&self) -> Result<DesiredGraphNode> {
        Ok(DesiredGraphNode::Object {
            capability: self.capability,
            name: ObjectName::new(&self.name)?,
            provider: ProviderName::new(self.capability.provider())?,
        })
    }

    pub async fn delete_reconciliation_steps(
        &self,
        graph_path: PathBuf,
    ) -> Vec<GraphExecutionStep> {
        let object = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = GraphStore::new(graph_path);
            let old_graph = store.load_desired_graph()?;
            let mut new_graph = old_graph.clone();
            let object_ref = ObjectRef::new(format!("{}/{}", object.capability, object.name))?;
            new_graph.nodes.remove(&object.graph_node()?);
            new_graph.nodes.retain(|node| {
                !matches!(node, DesiredGraphNode::Binding { object, .. } if object == &object_ref)
            });
            Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerBinding {
    pub capability: Capability,
    pub object_name: String,
    pub worker: String,
    pub binding: String,
    pub access: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ObjectBindingRef {
    pub worker: String,
    pub binding: String,
    pub access: String,
}

impl WorkerBinding {
    pub fn graph_node(&self) -> Result<DesiredGraphNode> {
        Ok(DesiredGraphNode::Binding {
            worker: WorkerId::new(&self.worker)?,
            name: BindingName::new(&self.binding)?,
            object: ObjectRef::new(format!("{}/{}", self.capability, self.object_name))?,
        })
    }

    pub async fn reconciliation_steps(&self, graph_path: PathBuf) -> Vec<GraphExecutionStep> {
        let binding = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = GraphStore::new(graph_path);
            let old_graph = store.load_desired_graph()?;
            let mut new_graph = old_graph.clone();
            new_graph.nodes.insert(binding.graph_node()?);
            Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    }

    pub async fn delete_reconciliation_steps(
        &self,
        graph_path: PathBuf,
    ) -> Vec<GraphExecutionStep> {
        let binding = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = GraphStore::new(graph_path);
            let old_graph = store.load_desired_graph()?;
            let mut new_graph = old_graph.clone();
            new_graph.nodes.remove(&binding.graph_node()?);
            Ok::<_, GumgumError>(GraphActionPlanner::plan_transition(&old_graph, &new_graph).steps)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesiredProvider {
    pub name: String,
    pub capability: Capability,
}

#[derive(Clone, Debug)]
pub struct GraphStore {
    path: PathBuf,
}

impl GraphStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn init(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                GumgumError::structured(
                    Subsystem::Config,
                    ErrorCode::Io,
                    "could not create graph directory",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        }
        let conn = self.open()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS desired_providers (
                name TEXT PRIMARY KEY,
                capability TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS desired_deployments (
                worker TEXT PRIMARY KEY,
                image TEXT NOT NULL,
                container TEXT NOT NULL,
                route TEXT,
                port INTEGER NOT NULL,
                health TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS global_objects (
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                namespace TEXT NOT NULL,
                root_domain TEXT NOT NULL,
                dns TEXT NOT NULL,
                provider TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'ready',
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(kind, name)
            );
            CREATE TABLE IF NOT EXISTS bindings (
                object_kind TEXT NOT NULL,
                object_name TEXT NOT NULL,
                worker TEXT NOT NULL,
                binding TEXT NOT NULL,
                access TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(worker, binding)
            );
            CREATE TABLE IF NOT EXISTS deployment_revisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                worker TEXT NOT NULL,
                image TEXT NOT NULL,
                container TEXT NOT NULL,
                route TEXT,
                port INTEGER NOT NULL,
                health TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS object_secrets (
                object_kind TEXT NOT NULL,
                object_name TEXT NOT NULL,
                field TEXT NOT NULL,
                env_name TEXT NOT NULL,
                secret_ref TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(object_kind, object_name, field)
            );
            CREATE TABLE IF NOT EXISTS reconciliation_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL DEFAULT 'reconciliation',
                operation_id TEXT,
                status TEXT NOT NULL,
                target TEXT NOT NULL,
                action TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .map_err(|source| self.error("could not initialize graph database", source))?;
        Ok(())
    }

    pub fn load_desired_graph(&self) -> Result<DesiredGraph> {
        self.init()?;
        let conn = self.open()?;
        let mut nodes = vec![
            DesiredGraphNode::Daemon {
                name: "gumgumd".to_owned(),
            },
            DesiredGraphNode::Provider {
                name: ProviderName::new("registry.platform")?,
                capability: Capability::Manual,
            },
            DesiredGraphNode::Provider {
                name: ProviderName::new("dnsmasq.platform")?,
                capability: Capability::Manual,
            },
            DesiredGraphNode::Provider {
                name: ProviderName::new("caddy.gateway")?,
                capability: Capability::Manual,
            },
            DesiredGraphNode::Provider {
                name: ProviderName::new("postgres.main")?,
                capability: Capability::Db,
            },
            DesiredGraphNode::Provider {
                name: ProviderName::new("redis.main")?,
                capability: Capability::Kv,
            },
        ];
        self.load_desired_providers(&conn, &mut nodes)?;
        self.load_desired_deployments(&conn, &mut nodes)?;
        self.load_desired_objects(&conn, &mut nodes)?;
        self.load_desired_bindings(&conn, &mut nodes)?;
        Ok(DesiredGraph::new(nodes))
    }

    pub fn record_reconcile_event(&self, event: &NewReconcileEvent) -> Result<ReconcileEventId> {
        self.init()?;
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO reconciliation_events (kind, operation_id, status, target, action, message, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)",
            params![
                event.kind.to_string(),
                event.operation_id,
                event.status.to_string(),
                event.target,
                event.action,
                event.message
            ],
        )
        .map_err(|source| self.error("could not record reconciliation event", source))?;
        Ok(ReconcileEventId::new(conn.last_insert_rowid()))
    }

    pub fn record_activity_event(
        &self,
        target: impl Into<String>,
        action: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<ReconcileEventId> {
        self.record_reconcile_event(&NewReconcileEvent {
            kind: ControlPlaneEventKind::Mutation,
            status: ReconcileEventStatus::Executed,
            operation_id: Some(new_operation_id("mutation")),
            target: target.into(),
            action: action.into(),
            message: message.into(),
        })
    }

    pub fn list_reconcile_events(&self, limit: u32) -> Result<Vec<ReconcileEvent>> {
        self.init()?;
        let conn = self.open()?;
        let limit = limit.clamp(1, 500);
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, operation_id, status, target, action, message, created_at
                 FROM reconciliation_events
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(|source| self.error("could not query reconciliation events", source))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                let kind: String = row.get(1)?;
                let operation_id: Option<String> = row.get(2)?;
                let status: String = row.get(3)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    kind,
                    operation_id,
                    status,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|source| self.error("could not read reconciliation events", source))?;
        let mut events = Vec::new();
        for row in rows {
            let (id, kind, operation_id, status, target, action, message, created_at) =
                row.map_err(|source| self.error("could not decode reconciliation event", source))?;
            events.push(ReconcileEvent {
                id: ReconcileEventId::new(id),
                kind: ControlPlaneEventKind::from_str(&kind)?,
                status: ReconcileEventStatus::from_str(&status)?,
                operation_id,
                target,
                action,
                message,
                created_at,
            });
        }
        Ok(events)
    }

    pub fn load_graph(&self) -> Result<(Vec<GraphNode>, Vec<GraphEdge>)> {
        self.init()?;
        let conn = self.open()?;
        let mut nodes = vec![
            GraphNode::new("gumgumd", "daemon", "gumgumd"),
            GraphNode::new("provider/registry.platform", "provider", "gumgum-registry"),
            GraphNode::new("provider/dnsmasq.platform", "provider", "gumgum-dnsmasq"),
            GraphNode::new("provider/caddy.gateway", "provider", "gumgum-caddy"),
            GraphNode::new("provider/postgres.main", "provider", "postgres.main"),
            GraphNode::new("provider/redis.main", "provider", "redis.main"),
        ];
        let mut edges = vec![
            GraphEdge::new("gumgumd", "provider/registry.platform", "owns"),
            GraphEdge::new("gumgumd", "provider/dnsmasq.platform", "owns"),
            GraphEdge::new("gumgumd", "provider/caddy.gateway", "owns"),
            GraphEdge::new("gumgumd", "provider/postgres.main", "owns"),
            GraphEdge::new("gumgumd", "provider/redis.main", "owns"),
        ];
        self.load_providers(&conn, &mut nodes, &mut edges)?;
        self.load_deployments(&conn, &mut nodes, &mut edges)?;
        self.load_objects(&conn, &mut nodes, &mut edges)?;
        self.load_bindings(&conn, &mut nodes, &mut edges)?;
        Ok((nodes, edges))
    }

    pub fn materialize_provider(&self, provider: &DesiredProvider) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO desired_providers (name, capability, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(name) DO UPDATE SET
               capability=excluded.capability,
               updated_at=CURRENT_TIMESTAMP",
            params![provider.name, provider.capability.to_string()],
        )
        .map_err(|source| self.error("could not materialize provider", source))?;
        self.record_activity_event(
            format!("provider/{}", provider.name),
            "provider.upsert",
            format!(
                "saved desired {} provider {}",
                provider.capability, provider.name
            ),
        )?;
        Ok(true)
    }

    pub fn object_bindings(&self, object: &GlobalObject) -> Result<Vec<ObjectBindingRef>> {
        self.init()?;
        let conn = self.open()?;
        let kind = object.capability.to_string();
        let mut stmt = conn
            .prepare(
                "SELECT worker, binding, access FROM bindings WHERE object_kind = ?1 AND object_name = ?2 ORDER BY worker, binding",
            )
            .map_err(|source| self.error("could not query object bindings", source))?;
        let rows = stmt
            .query_map(params![kind, object.name], |row| {
                Ok(ObjectBindingRef {
                    worker: row.get(0)?,
                    binding: row.get(1)?,
                    access: row.get(2)?,
                })
            })
            .map_err(|source| self.error("could not read object bindings", source))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|source| self.error("could not decode object bindings", source))
    }

    pub fn worker_bindings(&self, worker: &str) -> Result<Vec<ObjectBindingRef>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT object_kind || '/' || object_name, binding, access FROM bindings WHERE worker = ?1 ORDER BY binding",
            )
            .map_err(|source| self.error("could not query worker bindings", source))?;
        let rows = stmt
            .query_map(params![worker], |row| {
                Ok(ObjectBindingRef {
                    worker: row.get(0)?,
                    binding: row.get(1)?,
                    access: row.get(2)?,
                })
            })
            .map_err(|source| self.error("could not read worker bindings", source))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|source| self.error("could not decode worker bindings", source))
    }

    pub fn delete_object(&self, object: &GlobalObject) -> Result<bool> {
        self.init()?;
        let mut conn = self.open()?;
        let tx = conn
            .transaction()
            .map_err(|source| self.error("could not begin object delete transaction", source))?;
        let kind = object.capability.to_string();
        tx.execute(
            "DELETE FROM object_secrets WHERE object_kind = ?1 AND object_name = ?2",
            params![kind, object.name],
        )
        .map_err(|source| self.error("could not delete object secrets", source))?;
        let changed = tx
            .execute(
                "DELETE FROM global_objects WHERE kind = ?1 AND name = ?2",
                params![kind, object.name],
            )
            .map_err(|source| self.error("could not delete object", source))?;
        tx.commit()
            .map_err(|source| self.error("could not commit object delete transaction", source))?;
        if changed > 0 {
            self.record_activity_event(
                format!("object/{}/{}", object.capability, object.name),
                "object.delete",
                format!(
                    "deleted desired {} object {}",
                    object.capability, object.name
                ),
            )?;
        }
        Ok(changed > 0)
    }

    pub fn materialize_object(&self, object: &GlobalObject) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        let kind = object.capability.to_string();
        let dns = object_dns(&kind, &object.name, &object.root_domain);
        let provider = object.capability.provider();
        conn.execute(
            "INSERT INTO global_objects (kind, name, namespace, root_domain, dns, provider, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', CURRENT_TIMESTAMP)
             ON CONFLICT(kind, name) DO UPDATE SET
               namespace=excluded.namespace,
               root_domain=excluded.root_domain,
               dns=excluded.dns,
               provider=excluded.provider,
               status='ready',
               updated_at=CURRENT_TIMESTAMP",
            params![kind, object.name, object.namespace, object.root_domain, dns, provider],
        )
        .map_err(|source| self.error("could not materialize object", source))?;
        self.record_activity_event(
            format!("object/{}/{}", object.capability, object.name),
            "object.upsert",
            format!("saved desired {} object {}", object.capability, object.name),
        )?;
        Ok(true)
    }

    pub fn materialize_object_secret(
        &self,
        object_kind: &str,
        object_name: &str,
        field: &str,
        env_name: &str,
        secret_ref: &str,
        value: &str,
    ) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO object_secrets (object_kind, object_name, field, env_name, secret_ref, value, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
             ON CONFLICT(object_kind, object_name, field) DO UPDATE SET
               env_name=excluded.env_name,
               secret_ref=excluded.secret_ref,
               value=excluded.value,
               updated_at=CURRENT_TIMESTAMP",
            params![object_kind, object_name, field, env_name, secret_ref, value],
        )
        .map_err(|source| self.error("could not materialize object secret", source))?;
        self.record_activity_event(
            format!("object/{object_kind}/{object_name}"),
            "object_secret.upsert",
            format!("saved secret field {field} for {object_kind} object {object_name}"),
        )?;
        Ok(true)
    }

    pub fn object_secret(
        &self,
        object_kind: &str,
        object_name: &str,
        field: &str,
    ) -> Result<Option<String>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT value FROM object_secrets WHERE object_kind = ?1 AND object_name = ?2 AND field = ?3")
            .map_err(|source| self.error("could not query object secret", source))?;
        let mut rows = stmt
            .query(params![object_kind, object_name, field])
            .map_err(|source| self.error("could not read object secret", source))?;
        if let Some(row) = rows
            .next()
            .map_err(|source| self.error("could not decode object secret", source))?
        {
            Ok(Some(row.get(0).map_err(|source| {
                self.error("could not decode object secret", source)
            })?))
        } else {
            Ok(None)
        }
    }

    pub fn delete_binding(&self, binding: &WorkerBinding) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        let changed = conn
            .execute(
                "DELETE FROM bindings WHERE object_kind = ?1 AND object_name = ?2 AND worker = ?3 AND binding = ?4",
                params![
                    binding.capability.to_string(),
                    binding.object_name,
                    binding.worker,
                    binding.binding
                ],
            )
            .map_err(|source| self.error("could not delete binding", source))?;
        if changed > 0 {
            self.record_activity_event(
                format!("binding/{}/{}", binding.worker, binding.binding),
                "binding.delete",
                format!(
                    "deleted binding {} from worker {}",
                    binding.binding, binding.worker
                ),
            )?;
        }
        Ok(changed > 0)
    }

    pub fn materialize_binding(&self, binding: &WorkerBinding) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        conn.execute(
            "INSERT INTO bindings (object_kind, object_name, worker, binding, access, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
             ON CONFLICT(worker, binding) DO UPDATE SET
               object_kind=excluded.object_kind,
               object_name=excluded.object_name,
               access=excluded.access,
               updated_at=CURRENT_TIMESTAMP",
            params![
                binding.capability.to_string(),
                binding.object_name,
                binding.worker,
                binding.binding,
                binding.access
            ],
        )
        .map_err(|source| self.error("could not materialize binding", source))?;
        self.record_activity_event(
            format!("binding/{}/{}", binding.worker, binding.binding),
            "binding.upsert",
            format!(
                "saved binding {} for worker {}",
                binding.binding, binding.worker
            ),
        )?;
        Ok(true)
    }

    pub fn desired_deploy(&self, worker: &str) -> Result<Option<DesiredDeploy>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT worker, image, container, route, port, health FROM desired_deployments WHERE worker = ?1")
            .map_err(|source| self.error("could not query desired deployment", source))?;
        let mut rows = stmt
            .query(params![worker])
            .map_err(|source| self.error("could not read desired deployment", source))?;
        if let Some(row) = rows
            .next()
            .map_err(|source| self.error("could not decode desired deployment", source))?
        {
            Ok(Some(DesiredDeploy {
                worker: row
                    .get(0)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
                image: row
                    .get(1)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
                container: row
                    .get(2)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
                route: row
                    .get(3)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
                port: row
                    .get(4)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
                health: row
                    .get(5)
                    .map_err(|source| self.error("could not decode desired deployment", source))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_deploy(&self, worker: &str) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        let changed = conn
            .execute(
                "DELETE FROM desired_deployments WHERE worker = ?1",
                params![worker],
            )
            .map_err(|source| self.error("could not delete desired deployment", source))?;
        if changed > 0 {
            self.record_activity_event(
                format!("deployment/{worker}"),
                "deployment.delete",
                format!("deleted desired deployment {worker}"),
            )?;
        }
        Ok(changed > 0)
    }

    pub fn materialize_deploy(&self, request: &DesiredDeploy) -> Result<bool> {
        self.init()?;
        let mut conn = self.open()?;
        let tx = conn
            .transaction()
            .map_err(|source| self.error("could not begin graph transaction", source))?;
        tx.execute(
            "INSERT INTO deployment_revisions (worker, image, container, route, port, health)
             SELECT worker, image, container, route, port, health
             FROM desired_deployments
             WHERE worker = ?1
               AND (image != ?2 OR container != ?3 OR route != ?4 OR port != ?5 OR health != ?6)",
            params![
                request.worker,
                request.image,
                request.container,
                request.route,
                request.port,
                request.health
            ],
        )
        .map_err(|source| self.error("could not record deployment revision", source))?;
        tx.execute(
            "INSERT INTO desired_deployments (worker, image, container, route, port, health, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
             ON CONFLICT(worker) DO UPDATE SET
               image=excluded.image,
               container=excluded.container,
               route=excluded.route,
               port=excluded.port,
               health=excluded.health,
               updated_at=CURRENT_TIMESTAMP",
            params![request.worker, request.image, request.container, request.route, request.port, request.health],
        )
        .map_err(|source| self.error("could not materialize deployment", source))?;
        tx.commit()
            .map_err(|source| self.error("could not commit graph transaction", source))?;
        self.record_activity_event(
            format!("deployment/{}", request.worker),
            "deployment.upsert",
            format!("saved desired deployment {}", request.worker),
        )?;
        Ok(true)
    }

    pub fn latest_previous_deploy(&self, worker: &str) -> Result<Option<DesiredDeploy>> {
        Ok(self
            .latest_previous_revision(worker)?
            .map(|revision| revision.deploy))
    }

    pub fn latest_previous_revision(&self, worker: &str) -> Result<Option<DeploymentRevision>> {
        Ok(self.deployment_revisions(worker, 1)?.into_iter().next())
    }

    pub fn rollback_revision(
        &self,
        worker: &str,
        revision_id: Option<i64>,
    ) -> Result<Option<DeploymentRevision>> {
        match revision_id {
            Some(revision_id) => self.deployment_revision(worker, revision_id),
            None => self.latest_previous_revision(worker),
        }
    }

    pub fn deployment_revision(
        &self,
        worker: &str,
        revision_id: i64,
    ) -> Result<Option<DeploymentRevision>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT id, image, container, route, port, health, created_at FROM deployment_revisions WHERE worker = ?1 AND id = ?2")
            .map_err(|source| self.error("could not query deployment revision", source))?;
        let mut rows = stmt
            .query(params![worker, revision_id])
            .map_err(|source| self.error("could not read deployment revision", source))?;
        if let Some(row) = rows
            .next()
            .map_err(|source| self.error("could not decode deployment revision", source))?
        {
            Ok(Some(DeploymentRevision {
                id: row
                    .get(0)
                    .map_err(|source| self.error("could not decode deployment revision", source))?,
                deploy: DesiredDeploy {
                    worker: worker.to_owned(),
                    image: row.get(1).map_err(|source| {
                        self.error("could not decode deployment revision", source)
                    })?,
                    container: row.get(2).map_err(|source| {
                        self.error("could not decode deployment revision", source)
                    })?,
                    route: row.get(3).map_err(|source| {
                        self.error("could not decode deployment revision", source)
                    })?,
                    port: row.get::<_, i64>(4).map_err(|source| {
                        self.error("could not decode deployment revision", source)
                    })? as u16,
                    health: row.get(5).map_err(|source| {
                        self.error("could not decode deployment revision", source)
                    })?,
                },
                created_at: row
                    .get(6)
                    .map_err(|source| self.error("could not decode deployment revision", source))?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_deployment_revision(&self, worker: &str, revision_id: i64) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        let deleted = conn
            .execute(
                "DELETE FROM deployment_revisions WHERE worker = ?1 AND id = ?2",
                params![worker, revision_id],
            )
            .map_err(|source| self.error("could not delete deployment revision", source))?
            > 0;
        if deleted {
            self.record_activity_event(
                format!("deployment/{worker}/revision/{revision_id}"),
                "deployment_revision.delete",
                format!("deleted deployment revision {revision_id} for {worker}"),
            )?;
        }
        Ok(deleted)
    }

    pub fn deployment_revisions(
        &self,
        worker: &str,
        limit: u32,
    ) -> Result<Vec<DeploymentRevision>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT id, image, container, route, port, health, created_at FROM deployment_revisions WHERE worker = ?1 ORDER BY id DESC LIMIT ?2")
            .map_err(|source| self.error("could not query deployment revisions", source))?;
        let rows = stmt
            .query_map(params![worker, limit], |row| {
                Ok(DeploymentRevision {
                    id: row.get(0)?,
                    deploy: DesiredDeploy {
                        worker: worker.to_owned(),
                        image: row.get(1)?,
                        container: row.get(2)?,
                        route: row.get(3)?,
                        port: row.get::<_, i64>(4)? as u16,
                        health: row.get(5)?,
                    },
                    created_at: row.get(6)?,
                })
            })
            .map_err(|source| self.error("could not read deployment revisions", source))?;
        let mut revisions = Vec::new();
        for row in rows {
            revisions.push(
                row.map_err(|source| self.error("could not decode deployment revision", source))?,
            );
        }
        Ok(revisions)
    }

    pub fn binding_env(&self, worker: &str) -> Result<Vec<(String, String)>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT b.binding, b.object_kind, b.object_name, o.dns
                 FROM bindings b
                 JOIN global_objects o ON o.kind = b.object_kind AND o.name = b.object_name
                 WHERE b.worker = ?1
                 ORDER BY b.binding",
            )
            .map_err(|source| self.error("could not query binding env", source))?;
        let rows = stmt
            .query_map(params![worker], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|source| self.error("could not read binding env", source))?;
        let mut env = Vec::new();
        for row in rows {
            let (binding, kind, name, dns) =
                row.map_err(|source| self.error("could not decode binding env", source))?;
            let secret = if kind == "db" {
                self.object_secret(&kind, &name, "password")?
            } else {
                None
            };
            env.extend(binding_values(&kind, &binding, &name, &dns, secret));
        }
        Ok(env)
    }

    fn load_desired_providers(
        &self,
        conn: &Connection,
        nodes: &mut Vec<DesiredGraphNode>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT name, capability FROM desired_providers ORDER BY name")
            .map_err(|source| self.error("could not query desired providers", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|source| self.error("could not read desired providers", source))?;
        for row in rows {
            let (name, capability) =
                row.map_err(|source| self.error("could not decode desired provider", source))?;
            nodes.push(DesiredGraphNode::Provider {
                name: ProviderName::new(&name)?,
                capability: Capability::from_str(&capability).unwrap_or(Capability::Manual),
            });
        }
        Ok(())
    }

    fn load_desired_deployments(
        &self,
        conn: &Connection,
        nodes: &mut Vec<DesiredGraphNode>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare(
                "SELECT worker, image, container, route, port, health FROM desired_deployments ORDER BY worker",
            )
            .map_err(|source| self.error("could not query desired deployments", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, u16>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|source| self.error("could not read desired deployments", source))?;
        for row in rows {
            let (worker, image, container, route, port, health) =
                row.map_err(|source| self.error("could not decode desired deployment", source))?;
            nodes.push(DesiredGraphNode::Deployment {
                worker: WorkerId::new(&worker)?,
                image: ImageName::new(&image)?,
                container: ContainerName::new(&container)?,
                route: route.as_deref().map(RouteHost::new).transpose()?,
                port: Port::new(port)?,
                health: HealthPath::new(&health)?,
            });
        }
        Ok(())
    }

    fn load_desired_objects(
        &self,
        conn: &Connection,
        nodes: &mut Vec<DesiredGraphNode>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT kind, name, provider FROM global_objects ORDER BY kind, name")
            .map_err(|source| self.error("could not query desired objects", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|source| self.error("could not read desired objects", source))?;
        for row in rows {
            let (kind, name, provider) =
                row.map_err(|source| self.error("could not decode desired object", source))?;
            nodes.push(DesiredGraphNode::Object {
                capability: Capability::from_str(&kind).unwrap_or(Capability::Manual),
                name: ObjectName::new(&name)?,
                provider: ProviderName::new(&provider)?,
            });
        }
        Ok(())
    }

    fn load_desired_bindings(
        &self,
        conn: &Connection,
        nodes: &mut Vec<DesiredGraphNode>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT object_kind, object_name, worker, binding FROM bindings ORDER BY worker, binding")
            .map_err(|source| self.error("could not query desired bindings", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|source| self.error("could not read desired bindings", source))?;
        for row in rows {
            let (kind, name, worker, binding) =
                row.map_err(|source| self.error("could not decode desired binding", source))?;
            nodes.push(DesiredGraphNode::Binding {
                worker: WorkerId::new(&worker)?,
                name: BindingName::new(&binding)?,
                object: ObjectRef::new(format!("{kind}/{name}"))?,
            });
        }
        Ok(())
    }

    fn load_providers(
        &self,
        conn: &Connection,
        nodes: &mut Vec<GraphNode>,
        edges: &mut Vec<GraphEdge>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT name, capability FROM desired_providers ORDER BY name")
            .map_err(|source| self.error("could not query graph providers", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|source| self.error("could not read graph providers", source))?;
        for row in rows {
            let (name, capability) =
                row.map_err(|source| self.error("could not decode graph provider", source))?;
            let id = format!("provider/{name}");
            nodes.push(GraphNode::new(
                &id,
                "provider",
                format!("{name} ({capability})"),
            ));
            edges.push(GraphEdge::new("gumgumd", &id, "owns"));
        }
        Ok(())
    }

    fn load_deployments(
        &self,
        conn: &Connection,
        nodes: &mut Vec<GraphNode>,
        edges: &mut Vec<GraphEdge>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare(
                "SELECT worker, image, container, route FROM desired_deployments ORDER BY worker",
            )
            .map_err(|source| self.error("could not query graph database", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|source| self.error("could not read graph rows", source))?;
        for row in rows {
            let (worker, image, container, route) =
                row.map_err(|source| self.error("could not decode graph row", source))?;
            let worker_id = format!("worker/{worker}");
            let image_id = format!("image/{worker}");
            let container_id = format!("container/{container}");
            let route_id = format!("route/{route}");
            nodes.push(GraphNode::new(&worker_id, "worker", &worker));
            nodes.push(GraphNode::new(&image_id, "image", &image));
            let (domain_scope, namespace) = image_scope(&image);
            let project_network_id = format!("network/gumgum-{domain_scope}-{namespace}-network");
            let domain_network_id = format!("network/gumgum-{domain_scope}-network");
            nodes.push(GraphNode::new(&container_id, "container", &container));
            nodes.push(GraphNode::new(
                &project_network_id,
                "network",
                format!("gumgum-{domain_scope}-{namespace}-network"),
            ));
            nodes.push(GraphNode::new(
                &domain_network_id,
                "network",
                format!("gumgum-{domain_scope}-network"),
            ));
            nodes.push(GraphNode::new(
                "network/gumgum-network",
                "network",
                "gumgum-network",
            ));
            nodes.push(GraphNode::new(&route_id, "route", &route));
            edges.push(GraphEdge::new("gumgumd", &worker_id, "owns"));
            edges.push(GraphEdge::new(&worker_id, &image_id, "created_from"));
            edges.push(GraphEdge::new(&worker_id, &container_id, "runs"));
            edges.push(GraphEdge::new(
                &container_id,
                &project_network_id,
                "attached_to",
            ));
            edges.push(GraphEdge::new(
                &project_network_id,
                &domain_network_id,
                "depends_on",
            ));
            edges.push(GraphEdge::new(
                &domain_network_id,
                "network/gumgum-network",
                "depends_on",
            ));
            edges.push(GraphEdge::new(&worker_id, &route_id, "owns"));
            edges.push(GraphEdge::new(
                "provider/registry.platform",
                &image_id,
                "backs",
            ));
            edges.push(GraphEdge::new(
                "provider/caddy.gateway",
                &route_id,
                "routes",
            ));
            edges.push(GraphEdge::new(&route_id, &container_id, "routes_to"));
        }
        Ok(())
    }

    fn load_objects(
        &self,
        conn: &Connection,
        nodes: &mut Vec<GraphNode>,
        edges: &mut Vec<GraphEdge>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT kind, name, dns, provider FROM global_objects ORDER BY kind, name")
            .map_err(|source| self.error("could not query graph objects", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|source| self.error("could not read graph objects", source))?;
        for row in rows {
            let (kind, name, dns, provider) =
                row.map_err(|source| self.error("could not decode graph object", source))?;
            let object_id = format!("{kind}/{name}");
            let provider_id = format!("provider/{provider}");
            nodes.push(GraphNode::new(
                &object_id,
                "global_object",
                format!("{kind}: {dns}"),
            ));
            edges.push(GraphEdge::new(&provider_id, &object_id, "backs"));
        }
        Ok(())
    }

    fn load_bindings(
        &self,
        conn: &Connection,
        nodes: &mut Vec<GraphNode>,
        edges: &mut Vec<GraphEdge>,
    ) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT object_kind, object_name, worker, binding, access FROM bindings ORDER BY worker, binding")
            .map_err(|source| self.error("could not query graph bindings", source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|source| self.error("could not read graph bindings", source))?;
        for row in rows {
            let (kind, name, worker, binding, access) =
                row.map_err(|source| self.error("could not decode graph binding", source))?;
            let binding_id = format!("binding/{worker}/{binding}");
            let worker_id = format!("worker/{worker}");
            let object_id = format!("{kind}/{name}");
            nodes.push(GraphNode::new(
                &binding_id,
                "binding",
                format!("{binding} ({access})"),
            ));
            edges.push(GraphEdge::new(&worker_id, &binding_id, "binds"));
            edges.push(GraphEdge::new(&binding_id, &object_id, "projects_as"));
        }
        Ok(())
    }

    fn open(&self) -> Result<Connection> {
        Connection::open(&self.path)
            .map_err(|source| self.error("could not open graph database", source))
    }

    fn error(&self, message: &'static str, source: rusqlite::Error) -> GumgumError {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, message)
            .likely_cause(source.to_string())
            .build()
    }
}

pub fn object_dns(kind: &str, name: &str, root_domain: &str) -> String {
    format!("{}.{kind}.{root_domain}", crate::sanitize_name(name))
}

pub fn connection_examples(kind: &str, name: &str, dns: &str) -> Vec<String> {
    match kind {
        "db" | "database" => vec![
            format!("psql postgres://{name}:<password>@{dns}:5432/{name}"),
            format!("pgAdmin host={dns} port=5432 database={name} username={name}"),
        ],
        "kv" => vec![
            format!("redis-cli -u redis://{dns}:6379/0"),
            format!("RedisInsight host={dns} port=6379 database=0"),
        ],
        _ => Vec::new(),
    }
}

pub fn provider_for_object(kind: &str) -> &'static str {
    Capability::from_str(kind)
        .unwrap_or(Capability::Manual)
        .provider()
}

pub fn projected_binding_env(
    capability: Capability,
    binding: &str,
    object_name: &str,
) -> Vec<(String, String)> {
    binding_values(
        capability.as_str(),
        binding,
        object_name,
        &object_dns(capability.as_str(), object_name, "local"),
        None,
    )
}

fn binding_values(
    kind: &str,
    binding: &str,
    name: &str,
    dns: &str,
    secret: Option<String>,
) -> Vec<(String, String)> {
    match Capability::from_str(kind).unwrap_or(Capability::Manual) {
        Capability::Db => {
            let credentials = provider_credentials("postgres.main")
                .unwrap_or_else(crate::ProviderCredentials::postgres_local_dev);
            let database = crate::sanitize_name(name);
            let username = if secret.is_some() {
                database.clone()
            } else {
                credentials.username
            };
            let password = secret.unwrap_or(credentials.password);
            vec![(
                binding.to_owned(),
                format!(
                    "postgres://{username}:{password}@gumgum-provider-postgres-main:5432/{database}"
                ),
            )]
        }
        Capability::Kv => {
            let credentials = provider_credentials("redis.main")
                .unwrap_or_else(crate::ProviderCredentials::redis_local_dev);
            vec![(
                binding.to_owned(),
                format!(
                    "redis://:{}@gumgum-provider-redis-main:6379/0",
                    credentials.password
                ),
            )]
        }
        Capability::Blob => {
            let credentials = provider_credentials("minio.main")
                .unwrap_or_else(crate::ProviderCredentials::minio_local_dev);
            vec![
                (
                    binding.to_owned(),
                    format!("s3://gumgum-provider-minio-main/{name}"),
                ),
                (
                    format!("{binding}_ENDPOINT"),
                    "http://gumgum-provider-minio-main:9000".to_owned(),
                ),
                (format!("{binding}_BUCKET"), crate::sanitize_name(name)),
                (format!("{binding}_ACCESS_KEY_ID"), credentials.username),
                (format!("{binding}_SECRET_ACCESS_KEY"), credentials.password),
                (format!("{binding}_FORCE_PATH_STYLE"), "true".to_owned()),
            ]
        }
        Capability::Queue => vec![
            (
                binding.to_owned(),
                format!(
                    "kafka://gumgum-provider-redpanda-main:9092/{}",
                    crate::sanitize_name(name)
                ),
            ),
            (
                format!("{binding}_BROKERS"),
                "gumgum-provider-redpanda-main:9092".to_owned(),
            ),
            (format!("{binding}_TOPIC"), crate::sanitize_name(name)),
        ],
        Capability::Observability => vec![
            (binding.to_owned(), format!("http://{dns}:4317")),
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
                format!("http://{dns}:4317"),
            ),
            ("OTEL_SERVICE_NAME".to_owned(), crate::sanitize_name(name)),
        ],
        _ => vec![(binding.to_owned(), binding_value(kind, name, dns))],
    }
}

fn provider_credentials(provider: &str) -> Option<crate::ProviderCredentials> {
    crate::ConfigStore::from_home_env()
        .and_then(|store| store.load_provider_credentials(provider))
        .ok()
        .flatten()
}

fn binding_value(kind: &str, name: &str, dns: &str) -> String {
    match Capability::from_str(kind).unwrap_or(Capability::Manual) {
        Capability::Db => {
            let credentials = provider_credentials("postgres.main")
                .unwrap_or_else(crate::ProviderCredentials::postgres_local_dev);
            format!(
                "postgres://{}:{}@{dns}:5432/{}",
                credentials.username,
                credentials.password,
                crate::sanitize_name(name)
            )
        }
        Capability::Kv => format!("redis://{dns}:6379/0"),
        Capability::Blob => format!("s3://{dns}/{name}"),
        Capability::Queue => format!("kafka://{dns}/{name}"),
        Capability::Secret => format!("onepassword://gumgum/{name}"),
        Capability::Observability | Capability::Manual => dns.to_owned(),
    }
}

fn image_scope(image: &str) -> (String, String) {
    let repo = image.split('/').collect::<Vec<_>>();
    if repo.len() >= 4 {
        (repo[1].to_owned(), repo[2].to_owned())
    } else {
        ("local".to_owned(), "root".to_owned())
    }
}

pub fn new_operation_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", crate::sanitize_name(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> GraphStore {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        GraphStore::new(std::env::temp_dir().join(format!("gumgum-graph-{name}-{nonce}.sqlite")))
    }

    fn has_node(nodes: &[GraphNode], id: &str, kind: &str) -> bool {
        nodes.iter().any(|node| node.id == id && node.kind == kind)
    }

    fn has_edge(edges: &[GraphEdge], from: &str, to: &str, kind: &str) -> bool {
        edges
            .iter()
            .any(|edge| edge.from == from && edge.to == to && edge.kind == kind)
    }

    #[test]
    fn reconciliation_events_are_append_only_and_newest_first() {
        let store = temp_store("reconcile-events");
        let first = store
            .record_reconcile_event(&NewReconcileEvent {
                kind: ControlPlaneEventKind::Reconciliation,
                operation_id: None,
                status: ReconcileEventStatus::Planned,
                target: "provider/vaultwarden.main".to_owned(),
                action: "ensure provider".to_owned(),
                message: "planned provider reconcile".to_owned(),
            })
            .unwrap();
        let second = store
            .record_reconcile_event(&NewReconcileEvent {
                kind: ControlPlaneEventKind::Reconciliation,
                operation_id: None,
                status: ReconcileEventStatus::Executed,
                target: "provider/vaultwarden.main".to_owned(),
                action: "ensure provider".to_owned(),
                message: "provider reconciled".to_owned(),
            })
            .unwrap();

        assert!(second.get() > first.get());
        let events = store.list_reconcile_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, second);
        assert_eq!(events[0].kind, ControlPlaneEventKind::Reconciliation);
        assert_eq!(events[0].status, ReconcileEventStatus::Executed);
        assert_eq!(events[1].id, first);
        assert_eq!(events[1].status, ReconcileEventStatus::Planned);
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn desired_state_mutations_are_recorded_as_activity_events() {
        let store = temp_store("activity-events");
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Db,
                name: "main".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();

        let events = store.list_reconcile_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ControlPlaneEventKind::Mutation);
        assert_eq!(events[0].status, ReconcileEventStatus::Executed);
        assert_eq!(events[0].target, "object/db/main");
        assert_eq!(events[0].action, "object.upsert");
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn binding_queries_support_safe_delete_guards() {
        let store = temp_store("binding-queries");
        let object = GlobalObject {
            capability: Capability::Kv,
            name: "user-counters".to_owned(),
            namespace: "visit-counter".to_owned(),
            root_domain: "example.test".to_owned(),
        };
        store.materialize_object(&object).unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Kv,
                object_name: "user-counters".to_owned(),
                worker: "api".to_owned(),
                binding: "USER_COUNTERS".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();

        assert_eq!(store.object_bindings(&object).unwrap().len(), 1);
        assert_eq!(store.object_bindings(&object).unwrap()[0].worker, "api");
        assert_eq!(
            store.worker_bindings("api").unwrap()[0].worker,
            "kv/user-counters"
        );
        assert!(store.worker_bindings("worker").unwrap().is_empty());
        store
            .delete_binding(&WorkerBinding {
                capability: Capability::Kv,
                object_name: "user-counters".to_owned(),
                worker: "api".to_owned(),
                binding: "USER_COUNTERS".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        assert!(store.object_bindings(&object).unwrap().is_empty());
        assert!(store.delete_object(&object).unwrap());
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn materialized_state_loads_as_graph_with_bindings_and_routes() {
        let store = temp_store("load");
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Db,
                name: "main".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Db,
                object_name: "main".to_owned(),
                worker: "api".to_owned(),
                binding: "DATABASE_URL".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        store
            .materialize_deploy(&DesiredDeploy {
                worker: "api".to_owned(),
                image: "127.0.0.1:55000/dev.leostera/peekaboo/api:1".to_owned(),
                container: "gumgum-dev-leostera-peekaboo-api".to_owned(),
                route: Some("api.peekaboo.leostera.test".to_owned()),
                port: 3000,
                health: "/healthz".to_owned(),
            })
            .unwrap();

        let (nodes, edges) = store.load_graph().unwrap();
        assert!(has_node(&nodes, "worker/api", "worker"));
        assert!(has_node(&nodes, "db/main", "global_object"));
        assert!(has_node(&nodes, "binding/api/DATABASE_URL", "binding"));
        assert!(has_node(
            &nodes,
            "route/api.peekaboo.leostera.test",
            "route"
        ));
        assert!(has_edge(
            &edges,
            "worker/api",
            "binding/api/DATABASE_URL",
            "binds"
        ));
        assert!(has_edge(
            &edges,
            "binding/api/DATABASE_URL",
            "db/main",
            "projects_as"
        ));
        assert!(has_edge(
            &edges,
            "route/api.peekaboo.leostera.test",
            "container/gumgum-dev-leostera-peekaboo-api",
            "routes_to"
        ));

        let desired = store.load_desired_graph().unwrap();
        store
            .materialize_provider(&DesiredProvider {
                name: "vaultwarden.main".to_owned(),
                capability: Capability::Secret,
            })
            .unwrap();
        assert!(desired.nodes.contains(&DesiredGraphNode::Provider {
            name: ProviderName::new("postgres.main").unwrap(),
            capability: Capability::Db,
        }));
        assert!(desired.nodes.contains(&DesiredGraphNode::Object {
            capability: Capability::Db,
            name: ObjectName::new("main").unwrap(),
            provider: ProviderName::new("postgres.main").unwrap(),
        }));
        assert!(desired.nodes.contains(&DesiredGraphNode::Binding {
            worker: WorkerId::new("api").unwrap(),
            name: BindingName::new("DATABASE_URL").unwrap(),
            object: ObjectRef::new("db/main").unwrap(),
        }));
        assert!(desired.nodes.contains(&DesiredGraphNode::Deployment {
            worker: WorkerId::new("api").unwrap(),
            image: ImageName::new("127.0.0.1:55000/dev.leostera/peekaboo/api:1").unwrap(),
            container: ContainerName::new("gumgum-dev-leostera-peekaboo-api").unwrap(),
            route: Some(RouteHost::new("api.peekaboo.leostera.test").unwrap()),
            port: Port::new(3000).unwrap(),
            health: HealthPath::new("/healthz").unwrap(),
        }));
        let desired = store.load_desired_graph().unwrap();
        assert!(desired.nodes.contains(&DesiredGraphNode::Provider {
            name: ProviderName::new("vaultwarden.main").unwrap(),
            capability: Capability::Secret,
        }));
        let (nodes, edges) = store.load_graph().unwrap();
        assert!(has_node(&nodes, "provider/vaultwarden.main", "provider"));
        assert!(has_edge(
            &edges,
            "gumgumd",
            "provider/vaultwarden.main",
            "owns"
        ));
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn binding_env_and_previous_deploy_use_materialized_graph_state() {
        let store = temp_store("env-revisions");
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Db,
                name: "main".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Db,
                object_name: "main".to_owned(),
                worker: "api".to_owned(),
                binding: "DATABASE_URL".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Kv,
                name: "sessions".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Kv,
                object_name: "sessions".to_owned(),
                worker: "api".to_owned(),
                binding: "SESSIONS".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        let env = store.binding_env("api").unwrap();
        assert!(env.contains(&(
            "DATABASE_URL".to_owned(),
            "postgres://gumgum:gumgum-local-dev@gumgum-provider-postgres-main:5432/main".to_owned()
        )));
        assert!(env.contains(&(
            "SESSIONS".to_owned(),
            "redis://:gumgum-local-dev@gumgum-provider-redis-main:6379/0".to_owned()
        )));
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Blob,
                name: "User Uploads".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Blob,
                object_name: "User Uploads".to_owned(),
                worker: "api".to_owned(),
                binding: "UPLOADS".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        let env = store.binding_env("api").unwrap();
        assert!(env.contains(&(
            "UPLOADS_ENDPOINT".to_owned(),
            "http://gumgum-provider-minio-main:9000".to_owned()
        )));
        assert!(env.contains(&("UPLOADS_BUCKET".to_owned(), "user-uploads".to_owned())));
        assert!(env.contains(&("UPLOADS_ACCESS_KEY_ID".to_owned(), "gumgum".to_owned())));
        assert!(env.contains(&(
            "UPLOADS_SECRET_ACCESS_KEY".to_owned(),
            "gumgum-local-dev".to_owned()
        )));
        assert!(env.contains(&("UPLOADS_FORCE_PATH_STYLE".to_owned(), "true".to_owned())));
        store
            .materialize_object(&GlobalObject {
                capability: Capability::Queue,
                name: "visit-events".to_owned(),
                namespace: "peekaboo".to_owned(),
                root_domain: "leostera.dev".to_owned(),
            })
            .unwrap();
        store
            .materialize_binding(&WorkerBinding {
                capability: Capability::Queue,
                object_name: "visit-events".to_owned(),
                worker: "api".to_owned(),
                binding: "VISIT_EVENTS_QUEUE".to_owned(),
                access: "read-write".to_owned(),
            })
            .unwrap();
        let env = store.binding_env("api").unwrap();
        assert!(env.contains(&(
            "VISIT_EVENTS_QUEUE_BROKERS".to_owned(),
            "gumgum-provider-redpanda-main:9092".to_owned()
        )));
        assert!(env.contains(&(
            "VISIT_EVENTS_QUEUE_TOPIC".to_owned(),
            "visit-events".to_owned()
        )));

        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "127.0.0.1:55000/dev.leostera/peekaboo/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("api.peekaboo.leostera.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        assert!(store.latest_previous_deploy("api").unwrap().is_none());
        let mut second = first.clone();
        second.image = "127.0.0.1:55000/dev.leostera/peekaboo/api:2".to_owned();
        second.container = "gumgum-api-v2".to_owned();
        second.route = Some("api-v2.peekaboo.leostera.test".to_owned());
        second.port = 4000;
        second.health = "/ready".to_owned();
        store.materialize_deploy(&second).unwrap();
        let previous = store.latest_previous_deploy("api").unwrap().unwrap();
        assert_eq!(previous.image, first.image);
        assert_eq!(previous.container, first.container);
        assert_eq!(previous.route, first.route);
        assert_eq!(previous.port, first.port);
        assert_eq!(previous.health, first.health);
        let previous_revision = store.latest_previous_revision("api").unwrap().unwrap();
        assert_eq!(previous_revision.deploy.image, first.image);
        assert!(previous_revision.id > 0);
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn deployment_env_keys_do_not_replace_each_other() {
        let store = temp_store("env-deployments");
        let preview = DesiredDeploy {
            worker: "api@preview".to_owned(),
            image: "registry/api:preview".to_owned(),
            container: "gumgum-api-preview".to_owned(),
            route: Some("api-preview.example.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        let release = DesiredDeploy {
            worker: "api@release".to_owned(),
            image: "registry/api:release".to_owned(),
            container: "gumgum-api-release".to_owned(),
            route: Some("api.example.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };

        store.materialize_deploy(&preview).unwrap();
        store.materialize_deploy(&release).unwrap();

        assert_eq!(
            store
                .desired_deploy("api@preview")
                .unwrap()
                .unwrap()
                .container,
            "gumgum-api-preview"
        );
        assert_eq!(
            store
                .desired_deploy("api@release")
                .unwrap()
                .unwrap()
                .container,
            "gumgum-api-release"
        );
        assert!(
            store
                .latest_previous_deploy("api@preview")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .latest_previous_deploy("api@release")
                .unwrap()
                .is_none()
        );
        let desired = store.load_desired_graph().unwrap();
        assert!(desired.nodes.contains(&DesiredGraphNode::Deployment {
            worker: WorkerId::new("api-preview").unwrap(),
            image: ImageName::new("registry/api:preview").unwrap(),
            container: ContainerName::new("gumgum-api-preview").unwrap(),
            route: Some(RouteHost::new("api-preview.example.test").unwrap()),
            port: Port::new(3000).unwrap(),
            health: HealthPath::new("/healthz").unwrap(),
        }));
        assert!(desired.nodes.contains(&DesiredGraphNode::Deployment {
            worker: WorkerId::new("api-release").unwrap(),
            image: ImageName::new("registry/api:release").unwrap(),
            container: ContainerName::new("gumgum-api-release").unwrap(),
            route: Some(RouteHost::new("api.example.test").unwrap()),
            port: Port::new(3000).unwrap(),
            health: HealthPath::new("/healthz").unwrap(),
        }));
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn deploy_revisions_record_non_image_changes() {
        let store = temp_store("non-image-revision");
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "127.0.0.1:55000/dev.leostera/peekaboo/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("api.peekaboo.leostera.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();

        let mut route_change = first.clone();
        route_change.route = Some("api-v2.peekaboo.leostera.test".to_owned());
        store.materialize_deploy(&route_change).unwrap();

        let previous = store.latest_previous_deploy("api").unwrap().unwrap();
        assert_eq!(previous.image, first.image);
        assert_eq!(previous.container, first.container);
        assert_eq!(previous.route, first.route);
        assert_eq!(previous.port, first.port);
        assert_eq!(previous.health, first.health);
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn observability_binding_projects_otel_env() {
        let env = binding_values(
            "observability",
            "TELEMETRY",
            "visit-counter",
            "otel.platform",
            None,
        );

        assert!(env.contains(&(
            "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
            "http://otel.platform:4317".to_owned()
        )));
        assert!(env.contains(&("OTEL_SERVICE_NAME".to_owned(), "visit-counter".to_owned())));
    }

    #[test]
    fn deployment_revisions_list_newest_first_with_metadata() {
        let store = temp_store("revision-list");
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: Some("api.example.test".to_owned()),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        let mut second = first.clone();
        second.image = "registry/api:2".to_owned();
        store.materialize_deploy(&second).unwrap();
        let mut third = second.clone();
        third.route = Some("api-v3.example.test".to_owned());
        store.materialize_deploy(&third).unwrap();

        let revisions = store.deployment_revisions("api", 10).unwrap();
        assert_eq!(revisions.len(), 2);
        assert!(revisions[0].id > revisions[1].id);
        assert_eq!(revisions[0].deploy.image, second.image);
        assert_eq!(revisions[0].deploy.route, second.route);
        assert_eq!(revisions[1].deploy.image, first.image);
        assert_eq!(revisions[1].deploy.route, first.route);
        assert!(!revisions[0].created_at.is_empty());

        let limited = store.deployment_revisions("api", 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].id, revisions[0].id);

        let selected = store
            .rollback_revision("api", Some(revisions[1].id))
            .unwrap()
            .unwrap();
        assert_eq!(selected.id, revisions[1].id);
        assert_eq!(selected.deploy.image, first.image);
        let latest = store.rollback_revision("api", None).unwrap().unwrap();
        assert_eq!(latest.id, revisions[0].id);
        assert!(store.rollback_revision("api", Some(-1)).unwrap().is_none());

        assert!(
            store
                .delete_deployment_revision("api", revisions[0].id)
                .unwrap()
        );
        assert!(
            store
                .rollback_revision("api", Some(revisions[0].id))
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .delete_deployment_revision("api", revisions[0].id)
                .unwrap()
        );
        let remaining = store.deployment_revisions("api", 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, revisions[1].id);
        let _ = fs::remove_file(store.path);
    }
}
