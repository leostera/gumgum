use gumgum_api::GraphNode;

pub(crate) struct GraphPresenter;

impl GraphPresenter {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn describe_node(&self, node: &GraphNode) -> String {
        match node.kind.as_str() {
            "image" => format!("image {}", node.label),
            "container" => format!("container {}", node.label),
            "network" => format!("network {}", node.label),
            "route" => format!("route {}", node.label),
            "binding" => format!("binding {}", node.label),
            "global_object" => format!("object {}", node.label),
            "worker" => format!("worker {}", node.label),
            "provider" => format!("provider {}", node.label),
            _ => format!("{} {}", node.kind, node.label),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_known_graph_node_kinds() {
        let presenter = GraphPresenter::new();
        assert_eq!(
            presenter.describe_node(&GraphNode::new("db/main", "global_object", "main")),
            "object main"
        );
        assert_eq!(
            presenter.describe_node(&GraphNode::new("x", "custom", "thing")),
            "custom thing"
        );
    }
}
