use gumgum_core::{
    BindingName, Capability, ContainerName, DesiredGraph, DesiredGraphNode, GraphActionPlanner,
    HealthPath, ImageName, ObjectName, ObjectRef, Port, ProviderName, RouteHost, WorkerId,
};
use proptest::prelude::*;

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

proptest! {
    #[test]
    fn graph_planning_is_deterministic_and_idempotent(ops in prop::collection::vec(op_strategy(), 0..80)) {
        let mut current = DesiredGraph::default();
        assert_graph_invariants(&current);

        for op in ops {
            let old = current.clone();
            apply_op(&mut current, op);
            assert_graph_invariants(&current);

            let first = GraphActionPlanner::plan_transition(&old, &current);
            let second = GraphActionPlanner::plan_transition(&old, &current);
            prop_assert_eq!(first, second);

            let settled = GraphActionPlanner::plan_transition(&current, &current);
            prop_assert!(settled.is_empty());
        }
    }
}
