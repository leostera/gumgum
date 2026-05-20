use gumgum_core::{
    BindingPlanInput, Capability, DeployPlanner as CoreDeployPlanner, PlanGraph, WorkerManifest,
    WorkerPlanInput,
};

pub(crate) struct DeployPlanner<'a> {
    manifest: &'a WorkerManifest,
}

impl<'a> DeployPlanner<'a> {
    pub(crate) fn new(manifest: &'a WorkerManifest) -> Self {
        Self { manifest }
    }

    pub(crate) fn graph(&self) -> PlanGraph {
        self.core().graph()
    }

    pub(crate) fn plan_lines(&self) -> Vec<String> {
        self.core().plan_lines()
    }

    fn core(&self) -> CoreDeployPlanner {
        CoreDeployPlanner::new(WorkerPlanInput {
            worker_name: self.manifest.worker.name.clone(),
            databases: self
                .manifest
                .database
                .iter()
                .map(|binding| BindingPlanInput {
                    capability: Capability::Db,
                    name: binding.name.clone(),
                    binding: binding.binding.clone(),
                })
                .collect(),
            kvs: self
                .manifest
                .kv
                .iter()
                .map(|binding| BindingPlanInput {
                    capability: Capability::Kv,
                    name: binding.name.clone(),
                    binding: binding.binding.clone(),
                })
                .collect(),
        })
    }
}
