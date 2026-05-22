use crate::{PlanGraph, ServerRecord, WorkerManifest, sanitize_name};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct DeploymentDescriptor {
    pub worker: String,
    pub build_context: Option<String>,
    pub image: String,
    pub container: String,
    pub port: u16,
    pub routes: Vec<String>,
    pub health_url: Option<String>,
    pub plan: Vec<String>,
    pub plan_graph: PlanGraph,
}

impl DeploymentDescriptor {
    pub fn from_manifest(
        path: &Path,
        manifest: &WorkerManifest,
        server: Option<&ServerRecord>,
        prod: bool,
    ) -> Self {
        Self::from_manifest_in_namespace(path, manifest, None, server, prod)
    }

    pub fn from_manifest_in_namespace(
        path: &Path,
        manifest: &WorkerManifest,
        namespace: Option<&str>,
        server: Option<&ServerRecord>,
        prod: bool,
    ) -> Self {
        let worker = manifest.worker.name.clone();
        let namespace = namespace
            .or_else(|| {
                manifest
                    .project
                    .as_ref()
                    .map(|project| project.namespace.as_str())
            })
            .unwrap_or("root");
        let domain_scope = server
            .map(|server| dns_scope(&server.root_domain))
            .unwrap_or_else(|| "local".to_owned());
        let revision = stable_deploy_revision(path, manifest);
        let namespace_slug = sanitize_name(namespace);
        let worker_slug = sanitize_name(&worker);
        let image =
            format!("127.0.0.1:55000/{domain_scope}/{namespace_slug}/{worker_slug}:{revision}");
        let container = format!(
            "gumgum-{}",
            sanitize_name(&format!("{domain_scope}-{namespace}-{worker_slug}"))
        );
        let routes = derived_routes(manifest, namespace, server, prod);
        let health_url = derived_routes(manifest, namespace, server, false)
            .first()
            .map(|route| format!("http://{route}{}", manifest.worker.ready_check_path()));
        let build_context = Some(resolve_build_context(path, manifest));
        let planner = crate::DeployPlanner::from_manifest(manifest);
        Self {
            worker,
            build_context,
            image,
            container,
            port: deploy_port(manifest),
            routes,
            health_url,
            plan: planner.plan_lines(),
            plan_graph: planner.graph(),
        }
    }
}

fn deploy_port(manifest: &WorkerManifest) -> u16 {
    manifest
        .ingress
        .iter()
        .find_map(|ingress| ingress.port)
        .or(manifest.worker.port)
        .unwrap_or(3000)
}

fn resolve_build_context(path: &Path, manifest: &WorkerManifest) -> String {
    let context = manifest.worker.build_context.as_deref().unwrap_or(".");
    let context_path = PathBuf::from(context);
    if context_path.is_absolute() {
        context_path.display().to_string()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(context_path)
            .display()
            .to_string()
    }
}

fn derived_routes(
    manifest: &WorkerManifest,
    namespace: &str,
    server: Option<&ServerRecord>,
    prod: bool,
) -> Vec<String> {
    if manifest.ingress.is_empty() {
        return Vec::new();
    }
    let worker = sanitize_name(&manifest.worker.name);
    let project = namespace_route_label(namespace);
    let Some(server) = server else {
        return manifest
            .ingress
            .iter()
            .filter_map(|ingress| ingress.local_domain.clone())
            .collect();
    };

    let mut routes = vec![format!("{worker}.{project}.{}", server.root_domain)];
    routes.extend(
        manifest
            .ingress
            .iter()
            .filter_map(|ingress| ingress.local_domain.clone())
            .filter(|route| route.ends_with(&server.root_domain)),
    );
    if prod {
        routes.extend(
            manifest
                .ingress
                .iter()
                .filter_map(|ingress| ingress.public_domain.clone())
                .filter(|route| route.ends_with(&server.root_domain)),
        );
        routes.extend(
            manifest
                .zone
                .iter()
                .map(|zone| format!("{worker}.{}", zone.name.trim_start_matches("*."))),
        );
    }
    routes
}

fn stable_deploy_revision(path: &Path, manifest: &WorkerManifest) -> String {
    let mut hash = Fnv64::default();
    hash.write(path.display().to_string().as_bytes());
    hash.write(manifest.worker.name.as_bytes());
    hash.write(
        manifest
            .worker
            .image
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hash.write(
        manifest
            .worker
            .build_context
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hash.write(
        manifest
            .worker
            .command
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    hash.write(&manifest.worker.port.unwrap_or_default().to_be_bytes());
    hash.write(
        manifest
            .worker
            .health
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    for ingress in &manifest.ingress {
        hash.write(&ingress.port.unwrap_or_default().to_be_bytes());
        hash.write(
            ingress
                .local_domain
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        hash.write(
            ingress
                .public_domain
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        hash.write(&[ingress.public as u8]);
    }
    format!("gg{:016x}", hash.finish())
}

#[derive(Default)]
struct Fnv64(u64);

impl Fnv64 {
    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

fn namespace_route_label(namespace: &str) -> String {
    namespace
        .rsplit('.')
        .find(|label| !label.trim().is_empty())
        .map(sanitize_name)
        .unwrap_or_else(|| "root".to_owned())
}

fn dns_scope(root_domain: &str) -> String {
    root_domain
        .trim_end_matches('.')
        .split('.')
        .rev()
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Ingress, Project, Worker, WorkerManifest, Zone};

    fn server() -> ServerRecord {
        ServerRecord {
            name: "starbase".to_owned(),
            host: "192.168.0.3".to_owned(),
            root_domain: "leostera.dev".to_owned(),
            test_domain: "leostera.test".to_owned(),
            health_url: "http://192.168.0.3:7777/healthz".to_owned(),
        }
    }

    fn manifest() -> WorkerManifest {
        WorkerManifest {
            project: Some(Project {
                namespace: "experiments".to_owned(),
            }),
            worker: Worker {
                name: "Hello World".to_owned(),
                image: None,
                build_context: Some("api".to_owned()),
                command: None,
                port: Some(8080),
                checks: Default::default(),
                health: Some("/ready".to_owned()),
            },
            zone: vec![Zone {
                name: "*.example.com".to_owned(),
            }],
            ingress: vec![Ingress {
                name: "local".to_owned(),
                protocol: "http".to_owned(),
                port: Some(8080),
                local_domain: Some("hello.local".to_owned()),
                public_domain: None,
                public: false,
            }],
            database: Vec::new(),
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Default::default(),
            observability: None,
            limits: None,
        }
    }

    #[test]
    fn derives_test_descriptor_from_server() {
        let server = server();
        let descriptor = DeploymentDescriptor::from_manifest(
            Path::new("apps/api/gumgum.toml"),
            &manifest(),
            Some(&server),
            false,
        );
        assert_eq!(descriptor.worker, "Hello World");
        assert_eq!(
            descriptor.container,
            "gumgum-dev-leostera-experiments-hello-world"
        );
        assert_eq!(descriptor.port, 8080);
        assert_eq!(
            descriptor.routes,
            vec!["hello-world.experiments.leostera.dev"]
        );
        assert_eq!(
            descriptor.health_url.as_deref(),
            Some("http://hello-world.experiments.leostera.dev/ready")
        );
        assert_eq!(descriptor.build_context.as_deref(), Some("apps/api/api"));
        assert!(
            descriptor
                .image
                .starts_with("127.0.0.1:55000/dev.leostera/experiments/hello-world:")
        );
    }

    #[test]
    fn derives_prod_routes_from_project_and_zones() {
        let server = server();
        let descriptor = DeploymentDescriptor::from_manifest(
            Path::new("gumgum.toml"),
            &manifest(),
            Some(&server),
            true,
        );
        assert_eq!(
            descriptor.routes,
            vec![
                "hello-world.experiments.leostera.dev".to_owned(),
                "hello-world.example.com".to_owned(),
            ]
        );
        assert_eq!(
            descriptor.health_url.as_deref(),
            Some("http://hello-world.experiments.leostera.dev/ready")
        );
    }

    #[test]
    fn deploy_revision_is_stable_for_unchanged_manifest() {
        let first = DeploymentDescriptor::from_manifest(
            Path::new("api/gumgum.toml"),
            &manifest(),
            Some(&server()),
            false,
        );
        let second = DeploymentDescriptor::from_manifest(
            Path::new("api/gumgum.toml"),
            &manifest(),
            Some(&server()),
            false,
        );

        assert_eq!(first.image, second.image);
        let tag = first.image.rsplit(':').next().unwrap();
        assert!(tag.starts_with("gg"));
        assert_eq!(tag.len(), 18);
    }

    #[test]
    fn preserves_local_ingress_without_server() {
        let descriptor =
            DeploymentDescriptor::from_manifest(Path::new("gumgum.toml"), &manifest(), None, false);
        assert_eq!(descriptor.routes, vec!["hello.local"]);
        assert_eq!(descriptor.container, "gumgum-local-experiments-hello-world");
    }

    #[test]
    fn background_worker_without_ingress_has_no_invented_external_routes() {
        let mut manifest = manifest();
        manifest.ingress.clear();
        manifest.worker.name = "queue worker".to_owned();
        manifest.worker.port = None;

        let descriptor = DeploymentDescriptor::from_manifest(
            Path::new("worker/gumgum.toml"),
            &manifest,
            Some(&server()),
            false,
        );

        assert!(descriptor.routes.is_empty());
        assert_eq!(descriptor.health_url, None);
        assert_eq!(descriptor.port, 3000);
    }
}
