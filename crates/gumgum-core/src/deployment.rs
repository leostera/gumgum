use crate::{PlanGraph, ServerRecord, WorkerManifest, default_project_name, sanitize_name};
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
        let worker = manifest.worker.name.clone();
        let namespace = manifest
            .project
            .as_ref()
            .map(|project| project.namespace.as_str())
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
        let routes = derived_routes(manifest, server, prod);
        let health_url = derived_routes(manifest, server, false)
            .first()
            .map(|route| {
                let display_route = server
                    .map(|server| {
                        let root_suffix = format!(".{}", server.root_domain);
                        if route.ends_with(&root_suffix) {
                            format!(
                                "{}.{test_domain}",
                                route.trim_end_matches(&root_suffix),
                                test_domain = server.test_domain
                            )
                        } else {
                            route.clone()
                        }
                    })
                    .unwrap_or_else(|| route.clone());
                format!(
                    "http://{display_route}{}",
                    manifest.worker.health.as_deref().unwrap_or("/healthz")
                )
            });
        let build_context = manifest.worker.build_context.as_ref().map(|context| {
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
        });
        let planner = crate::DeployPlanner::from_manifest(manifest);
        Self {
            worker,
            build_context,
            image,
            container,
            port: manifest.worker.port.unwrap_or(3000),
            routes,
            health_url,
            plan: planner.plan_lines(),
            plan_graph: planner.graph(),
        }
    }
}

fn derived_routes(
    manifest: &WorkerManifest,
    server: Option<&ServerRecord>,
    prod: bool,
) -> Vec<String> {
    let worker = sanitize_name(&manifest.worker.name);
    let project = manifest
        .project
        .as_ref()
        .map(|project| sanitize_name(&project.namespace))
        .unwrap_or_else(default_project_name);
    let Some(server) = server else {
        return manifest
            .ingress
            .iter()
            .map(|ingress| ingress.local_domain.clone())
            .collect();
    };

    if prod {
        let mut routes = vec![format!("{worker}.{project}.{}", server.root_domain)];
        routes.extend(
            manifest
                .zone
                .iter()
                .map(|zone| format!("{worker}.{}", zone.name.trim_start_matches("*."))),
        );
        routes
    } else {
        vec![format!("{worker}.{project}.{}", server.test_domain)]
    }
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
        hash.write(ingress.local_domain.as_bytes());
        hash.write(
            ingress
                .public_domain
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        hash.write(&[ingress.publish as u8]);
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
                health: Some("/ready".to_owned()),
            },
            zone: vec![Zone {
                name: "*.example.com".to_owned(),
            }],
            ingress: vec![Ingress {
                name: "local".to_owned(),
                protocol: "http".to_owned(),
                local_domain: "hello.local".to_owned(),
                public_domain: None,
                publish: false,
            }],
            database: Vec::new(),
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Vec::new(),
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
            vec!["hello-world.experiments.leostera.test"]
        );
        assert_eq!(
            descriptor.health_url.as_deref(),
            Some("http://hello-world.experiments.leostera.test/ready")
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
            Some("http://hello-world.experiments.leostera.test/ready")
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
}
