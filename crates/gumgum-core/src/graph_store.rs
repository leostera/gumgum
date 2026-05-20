use crate::{ErrorCode, GraphEdge, GraphNode, GumgumError, Result, Subsystem};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesiredDeploy {
    pub worker: String,
    pub image: String,
    pub container: String,
    pub route: String,
    pub port: u16,
    pub health: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GlobalObject {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub root_domain: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerBinding {
    pub object_kind: String,
    pub object_name: String,
    pub worker: String,
    pub binding: String,
    pub access: String,
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
            "CREATE TABLE IF NOT EXISTS desired_deployments (
                worker TEXT PRIMARY KEY,
                image TEXT NOT NULL,
                container TEXT NOT NULL,
                route TEXT NOT NULL,
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
                route TEXT NOT NULL,
                port INTEGER NOT NULL,
                health TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .map_err(|source| self.error("could not initialize graph database", source))
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
        self.load_deployments(&conn, &mut nodes, &mut edges)?;
        self.load_objects(&conn, &mut nodes, &mut edges)?;
        self.load_bindings(&conn, &mut nodes, &mut edges)?;
        Ok((nodes, edges))
    }

    pub fn materialize_object(&self, object: &GlobalObject) -> Result<bool> {
        self.init()?;
        let conn = self.open()?;
        let dns = object_dns(&object.kind, &object.name, &object.root_domain);
        let provider = provider_for_object(&object.kind);
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
            params![object.kind, object.name, object.namespace, object.root_domain, dns, provider],
        )
        .map_err(|source| self.error("could not materialize object", source))?;
        Ok(true)
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
                binding.object_kind,
                binding.object_name,
                binding.worker,
                binding.binding,
                binding.access
            ],
        )
        .map_err(|source| self.error("could not materialize binding", source))?;
        Ok(true)
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
             WHERE worker = ?1 AND image != ?2",
            params![request.worker, request.image],
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
        Ok(true)
    }

    pub fn latest_previous_deploy(&self, worker: &str) -> Result<Option<DesiredDeploy>> {
        self.init()?;
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT r.image, d.container, d.route, d.port, d.health FROM deployment_revisions r JOIN desired_deployments d ON d.worker = r.worker WHERE r.worker = ?1 ORDER BY r.id DESC LIMIT 1")
            .map_err(|source| self.error("could not query deployment revisions", source))?;
        let mut rows = stmt
            .query(params![worker])
            .map_err(|source| self.error("could not read deployment revisions", source))?;
        if let Some(row) = rows
            .next()
            .map_err(|source| self.error("could not decode deployment revision", source))?
        {
            Ok(Some(DesiredDeploy {
                worker: worker.to_owned(),
                image: row.get(0).map_err(sql_decode_error)?,
                container: row.get(1).map_err(sql_decode_error)?,
                route: row.get(2).map_err(sql_decode_error)?,
                port: row.get::<_, i64>(3).map_err(sql_decode_error)? as u16,
                health: row.get(4).map_err(sql_decode_error)?,
            }))
        } else {
            Ok(None)
        }
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
                let binding: String = row.get(0)?;
                let kind: String = row.get(1)?;
                let name: String = row.get(2)?;
                let dns: String = row.get(3)?;
                Ok((binding, binding_value(&kind, &name, &dns)))
            })
            .map_err(|source| self.error("could not read binding env", source))?;
        let mut env = Vec::new();
        for row in rows {
            env.push(row.map_err(|source| self.error("could not decode binding env", source))?);
        }
        Ok(env)
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
    format!("{name}.{kind}.{root_domain}")
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
    match kind {
        "db" | "database" => "postgres.main",
        "kv" => "redis.main",
        "bucket" | "blob" => "minio.main",
        "queue" => "redpanda.main",
        _ => "manual.main",
    }
}

fn sql_decode_error(source: rusqlite::Error) -> GumgumError {
    GumgumError::structured(
        Subsystem::Config,
        ErrorCode::Io,
        "could not decode deployment revision",
    )
    .likely_cause(source.to_string())
    .build()
}

fn binding_value(kind: &str, name: &str, dns: &str) -> String {
    match kind {
        "db" | "database" => format!("postgres://{name}:gumgum@{dns}:5432/{name}"),
        "kv" => format!("redis://{dns}:6379/0"),
        "bucket" | "blob" => format!("s3://{dns}/{name}"),
        "queue" => format!("kafka://{dns}/{name}"),
        _ => dns.to_owned(),
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
