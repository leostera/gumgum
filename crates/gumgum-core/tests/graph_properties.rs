use gumgum_core::{
    BindingName, Capability, ContainerName, DesiredGraph, DesiredGraphNode, GraphActionPlanner,
    HealthPath, ImageName, ObjectName, ObjectRef, Port, ProviderName, RouteHost, WorkerId,
};
use proptest::prelude::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
enum ModelOp {
    AddProvider {
        capability: Capability,
    },
    AddObject {
        capability: Capability,
        name: String,
    },
    Bind {
        worker: String,
        binding: String,
        capability: Capability,
        object: String,
    },
    Deploy {
        worker: String,
    },
    RemoveBinding {
        worker: String,
        binding: String,
    },
    RemoveObject {
        capability: Capability,
        name: String,
    },
    RemoveDeploy {
        worker: String,
    },
}

fn capability_strategy() -> impl Strategy<Value = Capability> {
    prop_oneof![
        Just(Capability::Db),
        Just(Capability::Kv),
        Just(Capability::Blob),
        Just(Capability::Queue),
        Just(Capability::Secret),
    ]
}

fn name_strategy(prefix: &'static str) -> impl Strategy<Value = String> {
    (0usize..8).prop_map(move |idx| format!("{prefix}-{idx}"))
}

fn op_strategy() -> impl Strategy<Value = ModelOp> {
    prop_oneof![
        capability_strategy().prop_map(|capability| ModelOp::AddProvider { capability }),
        (capability_strategy(), name_strategy("obj"))
            .prop_map(|(capability, name)| { ModelOp::AddObject { capability, name } }),
        (
            name_strategy("worker"),
            name_strategy("binding"),
            capability_strategy(),
            name_strategy("obj"),
        )
            .prop_map(|(worker, binding, capability, object)| ModelOp::Bind {
                worker,
                binding,
                capability,
                object,
            }),
        name_strategy("worker").prop_map(|worker| ModelOp::Deploy { worker }),
        (name_strategy("worker"), name_strategy("binding"))
            .prop_map(|(worker, binding)| ModelOp::RemoveBinding { worker, binding }),
        (capability_strategy(), name_strategy("obj"))
            .prop_map(|(capability, name)| ModelOp::RemoveObject { capability, name }),
        name_strategy("worker").prop_map(|worker| ModelOp::RemoveDeploy { worker }),
    ]
}

fn provider_name(capability: Capability) -> ProviderName {
    ProviderName::new(capability.provider()).expect("capability provider names are valid")
}

fn object_node(capability: Capability, name: &str) -> DesiredGraphNode {
    DesiredGraphNode::Object {
        capability,
        name: ObjectName::new(name).expect("generated object names are valid"),
        provider: provider_name(capability),
    }
}

fn binding_node(
    worker: &str,
    binding: &str,
    capability: Capability,
    object: &str,
) -> DesiredGraphNode {
    DesiredGraphNode::Binding {
        worker: WorkerId::new(worker).expect("generated workers are valid"),
        name: BindingName::new(binding).expect("generated bindings are valid"),
        object: ObjectRef::new(format!("{capability}/{object}")).expect("object refs are valid"),
    }
}

fn deploy_node(worker: &str) -> DesiredGraphNode {
    DesiredGraphNode::Deployment {
        worker: WorkerId::new(worker).expect("generated workers are valid"),
        image: ImageName::new(format!("example/{worker}:latest")).expect("image is valid"),
        container: ContainerName::new(format!("gumgum-{worker}")).expect("container is valid"),
        route: RouteHost::new(format!("{worker}.example.test")).expect("route is valid"),
        port: Port::new(3000).expect("port is valid"),
        health: HealthPath::new("/healthz").expect("health is valid"),
    }
}

fn provider_node(capability: Capability) -> DesiredGraphNode {
    DesiredGraphNode::Provider {
        name: provider_name(capability),
        capability,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ReferenceModel {
    providers: BTreeSet<Capability>,
    objects: BTreeSet<(Capability, String)>,
    bindings: BTreeSet<(String, String, Capability, String)>,
    deploys: BTreeSet<String>,
}

impl ReferenceModel {
    fn apply(&mut self, op: ModelOp) {
        match op {
            ModelOp::AddProvider { capability } => {
                self.providers.insert(capability);
            }
            ModelOp::AddObject { capability, name } => {
                self.providers.insert(capability);
                self.objects.insert((capability, name));
            }
            ModelOp::Bind {
                worker,
                binding,
                capability,
                object,
            } => {
                self.providers.insert(capability);
                self.objects.insert((capability, object.clone()));
                self.bindings.insert((worker, binding, capability, object));
            }
            ModelOp::Deploy { worker } => {
                self.deploys.insert(worker);
            }
            ModelOp::RemoveBinding { worker, binding } => {
                self.bindings
                    .retain(|(existing_worker, existing_binding, _, _)| {
                        existing_worker != &worker || existing_binding != &binding
                    });
            }
            ModelOp::RemoveObject { capability, name } => {
                self.objects.remove(&(capability, name.clone()));
                self.bindings.retain(|(_, _, binding_capability, object)| {
                    binding_capability != &capability || object != &name
                });
            }
            ModelOp::RemoveDeploy { worker } => {
                self.deploys.remove(&worker);
            }
        }
    }

    fn desired_graph(&self) -> DesiredGraph {
        let nodes = self
            .providers
            .iter()
            .map(|capability| provider_node(*capability))
            .chain(
                self.objects
                    .iter()
                    .map(|(capability, name)| object_node(*capability, name)),
            )
            .chain(
                self.bindings
                    .iter()
                    .map(|(worker, binding, capability, object)| {
                        binding_node(worker, binding, *capability, object)
                    }),
            )
            .chain(self.deploys.iter().map(|worker| deploy_node(worker)));
        DesiredGraph::new(nodes)
    }
}

fn apply_op(graph: &mut DesiredGraph, op: ModelOp) {
    match op {
        ModelOp::AddProvider { capability } => {
            graph.nodes.insert(provider_node(capability));
        }
        ModelOp::AddObject { capability, name } => {
            graph.nodes.insert(provider_node(capability));
            graph.nodes.insert(object_node(capability, &name));
        }
        ModelOp::Bind {
            worker,
            binding,
            capability,
            object,
        } => {
            graph.nodes.insert(provider_node(capability));
            graph.nodes.insert(object_node(capability, &object));
            graph
                .nodes
                .insert(binding_node(&worker, &binding, capability, &object));
        }
        ModelOp::Deploy { worker } => {
            graph.nodes.insert(deploy_node(&worker));
        }
        ModelOp::RemoveBinding { worker, binding } => {
            graph.nodes.retain(|node| {
                !matches!(node, DesiredGraphNode::Binding { worker: w, name, .. }
                    if w.as_str() == worker && name.as_str() == binding)
            });
        }
        ModelOp::RemoveObject { capability, name } => {
            graph.nodes.remove(&object_node(capability, &name));
            let object_ref = format!("{capability}/{name}");
            graph.nodes.retain(|node| {
                !matches!(node, DesiredGraphNode::Binding { object, .. }
                    if object.as_str() == object_ref)
            });
        }
        ModelOp::RemoveDeploy { worker } => {
            graph.nodes.retain(|node| {
                !matches!(node, DesiredGraphNode::Deployment { worker: w, .. } if w.as_str() == worker)
            });
        }
    }
}

fn assert_graph_invariants(graph: &DesiredGraph) {
    for node in &graph.nodes {
        if let DesiredGraphNode::Binding { object, .. } = node {
            assert!(
                graph.nodes.iter().any(|candidate| matches!(candidate, DesiredGraphNode::Object { capability, name, .. }
                    if object.as_str() == format!("{capability}/{name}"))),
                "binding points at missing object {object} in {graph:?}"
            );
        }
    }
}

#[test]
fn regression_missing_object_binding_is_removable() {
    let invalid = DesiredGraph::new([binding_node(
        "worker-regression",
        "DATABASE_URL",
        Capability::Db,
        "missing-db",
    )]);
    let empty = DesiredGraph::default();

    let plan = GraphActionPlanner::plan_transition(&invalid, &empty);
    assert!(!plan.actions.is_empty());
    assert!(GraphActionPlanner::plan_transition(&empty, &empty).is_empty());
}

#[test]
fn regression_conflicting_binding_id_is_deterministic() {
    let invalid = DesiredGraph::new([
        object_node(Capability::Kv, "left-cache"),
        object_node(Capability::Kv, "right-cache"),
        binding_node("api", "USER_COUNTERS", Capability::Kv, "left-cache"),
        binding_node("api", "USER_COUNTERS", Capability::Kv, "right-cache"),
    ]);
    let empty = DesiredGraph::default();

    assert_eq!(
        GraphActionPlanner::plan_transition(&empty, &invalid),
        GraphActionPlanner::plan_transition(&empty, &invalid)
    );
}

proptest! {
    #[test]
    fn graph_planning_is_deterministic_and_idempotent(ops in prop::collection::vec(op_strategy(), 0..80)) {
        let mut current = DesiredGraph::default();
        let mut reference = ReferenceModel::default();
        assert_graph_invariants(&current);

        for op in ops {
            let old = current.clone();
            let reference_op = op.clone();
            apply_op(&mut current, op);
            reference.apply(reference_op);
            prop_assert_eq!(&current, &reference.desired_graph());
            assert_graph_invariants(&current);

            let first = GraphActionPlanner::plan_transition(&old, &current);
            let second = GraphActionPlanner::plan_transition(&old, &current);
            prop_assert_eq!(first, second);

            let settled = GraphActionPlanner::plan_transition(&current, &current);
            prop_assert!(settled.is_empty());
        }
    }

    #[test]
    fn invalid_graph_shapes_do_not_panic_or_mutate_reference(
        worker in name_strategy("worker"),
        binding in name_strategy("binding"),
        capability in capability_strategy(),
        object in name_strategy("missing"),
    ) {
        let invalid = DesiredGraph::new([binding_node(&worker, &binding, capability, &object)]);
        let empty = DesiredGraph::default();

        let first = GraphActionPlanner::plan_transition(&empty, &invalid);
        let second = GraphActionPlanner::plan_transition(&empty, &invalid);
        prop_assert_eq!(first, second);

        let remove = GraphActionPlanner::plan_transition(&invalid, &empty);
        prop_assert!(!remove.actions.is_empty());
        prop_assert!(empty.nodes.is_empty());
    }

    #[test]
    fn conflicting_binding_ids_are_still_deterministic_and_removable(
        worker in name_strategy("worker"),
        binding in name_strategy("binding"),
        left in name_strategy("left"),
        right in name_strategy("right"),
    ) {
        prop_assume!(left != right);
        let invalid = DesiredGraph::new([
            object_node(Capability::Kv, &left),
            object_node(Capability::Kv, &right),
            binding_node(&worker, &binding, Capability::Kv, &left),
            binding_node(&worker, &binding, Capability::Kv, &right),
        ]);
        let empty = DesiredGraph::default();

        let first = GraphActionPlanner::plan_transition(&empty, &invalid);
        let second = GraphActionPlanner::plan_transition(&empty, &invalid);
        prop_assert_eq!(&first, &second);
        prop_assert!(first.actions.len() >= 4);

        let remove = GraphActionPlanner::plan_transition(&invalid, &empty);
        prop_assert!(!remove.actions.is_empty());
    }
}
