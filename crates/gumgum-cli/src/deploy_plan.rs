use gumgum_core::{PlanEdge, PlanGraph, PlanNode};
use gumgum_manifest::WorkerManifest;

pub(crate) struct DeployPlanner<'a> {
    manifest: &'a WorkerManifest,
}

impl<'a> DeployPlanner<'a> {
    pub(crate) fn new(manifest: &'a WorkerManifest) -> Self {
        Self { manifest }
    }

    pub(crate) fn graph(&self) -> PlanGraph {
        let worker = &self.manifest.worker.name;
        let mut graph = MutablePlanGraph::new(worker);

        for db in &self.manifest.database {
            graph.add_binding(worker, "db", &db.name, db.binding.as_deref());
        }
        for kv in &self.manifest.kv {
            graph.add_binding(worker, "kv", &kv.name, kv.binding.as_deref());
        }

        graph.finish()
    }

    pub(crate) fn plan_lines(&self) -> Vec<String> {
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
                node(
                    "source/manifests",
                    "source",
                    "gumgum.toml files",
                    "collect manifest desired state",
                ),
                node(
                    "actual/containers",
                    "source",
                    "docker state",
                    "collect actual container state",
                ),
                node(
                    "provider/registry.platform",
                    "provider",
                    "registry.platform",
                    "ensure local registry provider is running",
                ),
                node(
                    &format!("image/{worker}"),
                    "image",
                    worker,
                    "build and push worker image",
                ),
                node(
                    &format!("container/{worker}"),
                    "container",
                    worker,
                    "reconcile worker container",
                ),
                node(
                    &format!("health/{worker}"),
                    "health_check",
                    worker,
                    "verify health check and routes",
                ),
            ],
            edges: vec![
                edge(
                    "source/manifests",
                    &format!("image/{worker}"),
                    "desired_state",
                ),
                edge(
                    "actual/containers",
                    &format!("container/{worker}"),
                    "actual_state",
                ),
                edge(
                    "provider/registry.platform",
                    &format!("image/{worker}"),
                    "backs",
                ),
                edge(
                    &format!("image/{worker}"),
                    &format!("container/{worker}"),
                    "created_from",
                ),
                edge(
                    &format!("container/{worker}"),
                    &format!("health/{worker}"),
                    "has_health_check",
                ),
            ],
        }
    }

    fn add_binding(&mut self, worker: &str, kind: &str, object: &str, binding: Option<&str>) {
        let provider = provider_for_object(kind);
        let object_id = format!("{kind}/{object}");
        self.nodes.push(node(
            &format!("provider/{provider}"),
            "provider",
            provider,
            "ensure provider is running",
        ));
        self.nodes.push(node(
            &object_id,
            "global_object",
            object,
            "ensure global object exists",
        ));
        self.edges
            .push(edge("source/manifests", &object_id, "desired_state"));
        self.edges
            .push(edge(&format!("provider/{provider}"), &object_id, "backs"));
        if let Some(binding) = binding {
            let binding_id = format!("binding/{worker}/{binding}");
            self.nodes.push(node(
                &binding_id,
                "binding",
                binding,
                "ensure worker-local binding exists",
            ));
            self.edges
                .push(edge(&object_id, &binding_id, "projects_as"));
            self.edges.push(edge(
                &binding_id,
                &format!("container/{worker}"),
                "injects_into",
            ));
        }
    }

    fn finish(self) -> PlanGraph {
        PlanGraph::new(self.nodes, self.edges)
    }
}

fn node(id: &str, kind: &str, label: &str, action: &str) -> PlanNode {
    PlanNode::new(id, kind, label, action)
}

fn edge(from: &str, to: &str, kind: &str) -> PlanEdge {
    PlanEdge::new(from, to, kind)
}

fn provider_for_object(kind: &str) -> &'static str {
    match kind {
        "db" | "database" => "postgres.main",
        "kv" => "redis.main",
        "bucket" | "blob" => "minio.main",
        "queue" => "redpanda.main",
        _ => "manual.main",
    }
}
