use crate::{PlanGraph, ServerRecord, WorkerManifest};
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
        let revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let image = format!("127.0.0.1:55000/{domain_scope}/{namespace}/{worker}:{revision}");
        let container = format!(
            "gumgum-{}",
            sanitize_name(&format!("{domain_scope}-{namespace}-{worker}"))
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

fn default_project_name() -> String {
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

fn dns_scope(root_domain: &str) -> String {
    root_domain
        .trim_end_matches('.')
        .split('.')
        .rev()
        .collect::<Vec<_>>()
        .join(".")
}

fn sanitize_name(value: &str) -> String {
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
