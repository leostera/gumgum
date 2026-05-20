use crate::{Capability, ErrorCode, GraphEdge, GraphNode, GumgumError, Result, Subsystem};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkerBinding {
    pub capability: Capability,
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
                binding.capability.to_string(),
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
                let binding: String = row.get(0)?;
                let kind: String = row.get(1)?;
                let name: String = row.get(2)?;
                let dns: String = row.get(3)?;
                Ok(binding_values(&kind, &binding, &name, &dns))
            })
            .map_err(|source| self.error("could not read binding env", source))?;
        let mut env = Vec::new();
        for row in rows {
            env.extend(row.map_err(|source| self.error("could not decode binding env", source))?);
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

fn binding_values(kind: &str, binding: &str, name: &str, dns: &str) -> Vec<(String, String)> {
    match Capability::from_str(kind).unwrap_or(Capability::Manual) {
        Capability::Blob => {
            let credentials = crate::ProviderCredentials::minio_local_dev();
            vec![
                (binding.to_owned(), format!("s3://{dns}/{name}")),
                (format!("{binding}_ENDPOINT"), format!("http://{dns}:9000")),
                (format!("{binding}_BUCKET"), crate::sanitize_name(name)),
                (format!("{binding}_ACCESS_KEY_ID"), credentials.username),
                (format!("{binding}_SECRET_ACCESS_KEY"), credentials.password),
                (format!("{binding}_FORCE_PATH_STYLE"), "true".to_owned()),
            ]
        }
        _ => vec![(binding.to_owned(), binding_value(kind, name, dns))],
    }
}

fn binding_value(kind: &str, name: &str, dns: &str) -> String {
    match Capability::from_str(kind).unwrap_or(Capability::Manual) {
        Capability::Db => format!("postgres://{name}:gumgum@{dns}:5432/{name}"),
        Capability::Kv => format!("redis://{dns}:6379/0"),
        Capability::Blob => format!("s3://{dns}/{name}"),
        Capability::Queue => format!("kafka://{dns}/{name}"),
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
                route: "api.peekaboo.leostera.test".to_owned(),
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
        let _ = fs::remove_file(store.path);
    }

    #[test]
    fn binding_env_and_previous_deploy_use_materialized_graph_state() {
        let store = temp_store("env-revisions");
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
        assert_eq!(
            store.binding_env("api").unwrap(),
            vec![(
                "SESSIONS".to_owned(),
                "redis://sessions.kv.leostera.dev:6379/0".to_owned()
            )]
        );
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
            "http://user-uploads.blob.leostera.dev:9000".to_owned()
        )));
        assert!(env.contains(&("UPLOADS_BUCKET".to_owned(), "user-uploads".to_owned())));
        assert!(env.contains(&("UPLOADS_ACCESS_KEY_ID".to_owned(), "gumgum".to_owned())));
        assert!(env.contains(&(
            "UPLOADS_SECRET_ACCESS_KEY".to_owned(),
            "gumgum-local-dev".to_owned()
        )));
        assert!(env.contains(&("UPLOADS_FORCE_PATH_STYLE".to_owned(), "true".to_owned())));

        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "127.0.0.1:55000/dev.leostera/peekaboo/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: "api.peekaboo.leostera.test".to_owned(),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        assert!(store.latest_previous_deploy("api").unwrap().is_none());
        let mut second = first.clone();
        second.image = "127.0.0.1:55000/dev.leostera/peekaboo/api:2".to_owned();
        second.container = "gumgum-api-v2".to_owned();
        second.route = "api-v2.peekaboo.leostera.test".to_owned();
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
    fn deploy_revisions_record_non_image_changes() {
        let store = temp_store("non-image-revision");
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "127.0.0.1:55000/dev.leostera/peekaboo/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: "api.peekaboo.leostera.test".to_owned(),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();

        let mut route_change = first.clone();
        route_change.route = "api-v2.peekaboo.leostera.test".to_owned();
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
    fn deployment_revisions_list_newest_first_with_metadata() {
        let store = temp_store("revision-list");
        let first = DesiredDeploy {
            worker: "api".to_owned(),
            image: "registry/api:1".to_owned(),
            container: "gumgum-api".to_owned(),
            route: "api.example.test".to_owned(),
            port: 3000,
            health: "/healthz".to_owned(),
        };
        store.materialize_deploy(&first).unwrap();
        let mut second = first.clone();
        second.image = "registry/api:2".to_owned();
        store.materialize_deploy(&second).unwrap();
        let mut third = second.clone();
        third.route = "api-v3.example.test".to_owned();
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
        let _ = fs::remove_file(store.path);
    }
}
