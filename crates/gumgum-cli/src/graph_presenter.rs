use gumgum_api::{GraphEdge, GraphNode};

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

fn sanitize_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}
