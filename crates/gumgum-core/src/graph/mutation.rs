use crate::{DesiredGraph, DesiredGraphNode, GraphNodeId, ObjectRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "mutation")]
pub enum GraphMutation {
    UpsertNode { node: DesiredGraphNode },
    RemoveNode { node: DesiredGraphNode },
    RemoveNodeById { id: GraphNodeId },
    RemoveObjectAndBindings { object: ObjectRef },
}

impl GraphMutation {
    pub fn upsert_node(node: DesiredGraphNode) -> Self {
        Self::UpsertNode { node }
    }

    pub fn remove_node(node: DesiredGraphNode) -> Self {
        Self::RemoveNode { node }
    }

    pub fn remove_node_by_id(id: GraphNodeId) -> Self {
        Self::RemoveNodeById { id }
    }

    pub fn remove_object_and_bindings(object: ObjectRef) -> Self {
        Self::RemoveObjectAndBindings { object }
    }

    pub fn apply(&self, graph: &mut DesiredGraph) {
        match self {
            Self::UpsertNode { node } => {
                graph.nodes.replace(node.clone());
            }
            Self::RemoveNode { node } => {
                graph.nodes.remove(node);
            }
            Self::RemoveNodeById { id } => {
                graph.nodes.retain(|node| node.id() != id.as_str());
            }
            Self::RemoveObjectAndBindings { object } => {
                graph.nodes.retain(|node| match node {
                    DesiredGraphNode::Object {
                        capability, name, ..
                    } => format!("{capability}/{name}") != object.as_str(),
                    DesiredGraphNode::Binding { object: bound, .. } => bound != object,
                    _ => true,
                });
            }
        }
    }

    pub fn apply_all<'a>(
        graph: &DesiredGraph,
        mutations: impl IntoIterator<Item = &'a GraphMutation>,
    ) -> DesiredGraph {
        let mut graph = graph.clone();
        for mutation in mutations {
            mutation.apply(&mut graph);
        }
        graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BindingName, Capability, ObjectName, ProviderName, WorkerId};

    #[test]
    fn upsert_node_is_idempotent() {
        let node = DesiredGraphNode::Object {
            capability: Capability::Db,
            name: ObjectName::new("visits").unwrap(),
            provider: ProviderName::new("postgres.main").unwrap(),
        };
        let mutation = GraphMutation::upsert_node(node.clone());
        let mut graph = DesiredGraph::default();

        mutation.apply(&mut graph);
        mutation.apply(&mut graph);

        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.nodes.contains(&node));
    }

    #[test]
    fn remove_object_and_bindings_prunes_dependents() {
        let object = DesiredGraphNode::Object {
            capability: Capability::Kv,
            name: ObjectName::new("user-counters").unwrap(),
            provider: ProviderName::new("redis.main").unwrap(),
        };
        let binding = DesiredGraphNode::Binding {
            worker: WorkerId::new("api").unwrap(),
            name: BindingName::new("USER_COUNTERS").unwrap(),
            object: ObjectRef::new("kv/user-counters").unwrap(),
        };
        let unrelated = DesiredGraphNode::Object {
            capability: Capability::Db,
            name: ObjectName::new("visits").unwrap(),
            provider: ProviderName::new("postgres.main").unwrap(),
        };
        let mut graph = DesiredGraph::new([object, binding, unrelated.clone()]);

        GraphMutation::remove_object_and_bindings(ObjectRef::new("kv/user-counters").unwrap())
            .apply(&mut graph);

        assert_eq!(graph.nodes, DesiredGraph::new([unrelated]).nodes);
    }
}
