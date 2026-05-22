use crate::{PlanGraph, ServerRecord, WorkerManifest, sanitize_name};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

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
        Self::from_manifest_in_project(path, manifest, None, None, server, prod)
    }

    pub fn from_manifest_in_project(
        path: &Path,
        manifest: &WorkerManifest,
        project_name: Option<&str>,
        project_domain: Option<&str>,
        server: Option<&ServerRecord>,
        prod: bool,
    ) -> Self {
        let worker = manifest.worker.name.clone();
        let project_name = project_name
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
        let project_slug = sanitize_name(project_name);
        let worker_slug = sanitize_name(&worker);
        let image =
            format!("127.0.0.1:55000/{domain_scope}/{project_slug}/{worker_slug}:{revision}");
        let container = format!(
            "gumgum-{}",
            sanitize_name(&format!("{domain_scope}-{project_name}-{worker_slug}"))
        );
        let routes = derived_routes(manifest, project_name, project_domain, server, prod);
        let health_url = derived_routes(manifest, project_name, project_domain, server, false)
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
    project_name: &str,
    project_domain: Option<&str>,
    server: Option<&ServerRecord>,
    prod: bool,
) -> Vec<String> {
    if manifest.ingress.is_empty() {
        return Vec::new();
    }
    let worker = sanitize_name(&manifest.worker.name);
    let project = sanitize_name(project_name);
    let Some(server) = server else {
        return manifest
            .ingress
            .iter()
            .filter_map(|ingress| ingress.local_domain.clone())
            .collect();
    };

    let route_domain = if prod {
        project_domain.unwrap_or(&server.root_domain)
    } else {
        &server.root_domain
    };
    let mut routes = vec![format!("{worker}.{project}.{route_domain}")];
    routes.extend(
        manifest
            .ingress
            .iter()
            .filter_map(|ingress| ingress.local_domain.clone())
            .filter(|route| route.ends_with(route_domain)),
    );
    if prod {
        routes.extend(
            manifest
                .ingress
                .iter()
                .filter_map(|ingress| ingress.public_domain.clone())
                .filter(|route| route.ends_with(route_domain)),
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
    hash_build_context(path, manifest, &mut hash);
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

fn hash_build_context(path: &Path, manifest: &WorkerManifest, hash: &mut Fnv64) {
    let context = resolve_build_context(path, manifest);
    let context_path = Path::new(&context);
    let mut files = Vec::new();
    collect_context_files(context_path, context_path, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    for (relative_path, absolute_path) in files {
        hash.write(relative_path.to_string_lossy().as_bytes());
        if let Ok(contents) = fs::read(&absolute_path) {
            hash.write(&contents);
        }
    }
}

fn collect_context_files(root: &Path, current: &Path, files: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(metadata) = fs::symlink_metadata(current) else {
        return;
    };
    if metadata.is_file() {
        let relative = current.strip_prefix(root).unwrap_or(current).to_path_buf();
        files.push((relative, current.to_path_buf()));
        return;
    }
    if !metadata.is_dir() || should_skip_context_entry(current) {
        return;
    }
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        collect_context_files(root, &entry.path(), files);
    }
}

fn should_skip_context_entry(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | ".pytest_cache" | ".venv" | "__pycache__" | "node_modules" | "target")
    )
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
        let descriptor = DeploymentDescriptor::from_manifest_in_project(
            Path::new("gumgum.toml"),
            &manifest(),
            Some("hello"),
            Some("hello.example"),
            Some(&server),
            true,
        );
        assert_eq!(
            descriptor.routes,
            vec![
                "hello-world.hello.hello.example".to_owned(),
                "hello-world.example.com".to_owned(),
            ]
        );
        assert_eq!(
            descriptor.health_url.as_deref(),
            Some("http://hello-world.hello.leostera.dev/ready")
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
    fn deploy_revision_changes_when_build_context_changes() {
        let root = std::env::temp_dir().join(format!(
            "gumgum-deploy-revision-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let api_dir = root.join("api");
        fs::create_dir_all(api_dir.join("src")).unwrap();
        fs::write(api_dir.join("gumgum.toml"), "").unwrap();
        fs::write(api_dir.join("src/main.py"), "print('one')\n").unwrap();

        let mut manifest = manifest();
        manifest.worker.build_context = None;
        let before = DeploymentDescriptor::from_manifest(
            &api_dir.join("gumgum.toml"),
            &manifest,
            Some(&server()),
            false,
        );
        fs::write(api_dir.join("src/main.py"), "print('two')\n").unwrap();
        let after = DeploymentDescriptor::from_manifest(
            &api_dir.join("gumgum.toml"),
            &manifest,
            Some(&server()),
            false,
        );

        let _ = fs::remove_dir_all(root);
        assert_ne!(before.image, after.image);
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
