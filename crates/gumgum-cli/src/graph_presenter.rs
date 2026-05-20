use gumgum_api::{GraphEdge, GraphNode};
use gumgum_core::sanitize_name;

pub(crate) struct GraphPresenter;

impl GraphPresenter {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn mermaid(&self, nodes: &[GraphNode], edges: &[GraphEdge]) -> String {
        let mut graph = "graph TD\n".to_owned();
        for node in nodes {
            graph.push_str(&format!(
                "  {}[\"{}\"]\n",
                self.mermaid_id(&node.id),
                self.mermaid_label(&node.label)
            ));
        }
        for edge in edges {
            graph.push_str(&format!(
                "  {} -->|{}| {}\n",
                self.mermaid_id(&edge.from),
                edge.kind,
                self.mermaid_id(&edge.to)
            ));
        }
        graph
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

    fn mermaid_id(&self, value: &str) -> String {
        sanitize_name(value).replace('-', "_")
    }

    fn mermaid_label(&self, value: &str) -> String {
        value.replace('"', "\\\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_escapes_labels_and_sanitizes_ids() {
        let presenter = GraphPresenter::new();
        let nodes = vec![
            GraphNode::new("route/api.example.test", "route", "api \"quoted\" route"),
            GraphNode::new("container/api", "container", "api"),
        ];
        let edges = vec![GraphEdge::new(
            "route/api.example.test",
            "container/api",
            "routes_to",
        )];

        let graph = presenter.mermaid(&nodes, &edges);
        assert!(graph.contains("route_api_example_test[\"api \\\"quoted\\\" route\"]"));
        assert!(graph.contains("route_api_example_test -->|routes_to| container_api"));
    }

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
