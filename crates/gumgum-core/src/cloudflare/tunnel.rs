use std::collections::HashMap;

use crate::{ContainerRunSpec, DockerEngine, Result};

const CLOUDFLARED_CONTAINER: &str = "gumgum-cloudflared";
const CLOUDFLARED_IMAGE: &str = "cloudflare/cloudflared:latest";
const GUMGUM_NETWORK: &str = "gumgum-network";

pub async fn ensure_cloudflared(token: &str) -> Result<Vec<String>> {
    let docker = DockerEngine::local()?;
    if cloudflared_running_on_gumgum_network(&docker).await? {
        return Ok(vec![format!(
            "ensure Cloudflare connector container {CLOUDFLARED_CONTAINER}"
        )]);
    }

    let _ = docker.remove_container_force(CLOUDFLARED_CONTAINER).await;
    docker.pull_image(CLOUDFLARED_IMAGE).await?;
    docker
        .create_and_start_container(ContainerRunSpec {
            name: CLOUDFLARED_CONTAINER.to_owned(),
            image: CLOUDFLARED_IMAGE.to_owned(),
            network: GUMGUM_NETWORK.to_owned(),
            restart_unless_stopped: true,
            labels: HashMap::new(),
            env: vec![("TUNNEL_TOKEN".to_owned(), token.to_owned())],
            binds: Vec::new(),
            ports: Vec::new(),
            command: vec![
                "tunnel".to_owned(),
                "--no-autoupdate".to_owned(),
                "run".to_owned(),
            ],
            entrypoint: Vec::new(),
        })
        .await?;

    Ok(vec![format!(
        "started Cloudflare connector container {CLOUDFLARED_CONTAINER}"
    )])
}

async fn cloudflared_running_on_gumgum_network(docker: &DockerEngine) -> Result<bool> {
    Ok(docker
        .inspect_container(CLOUDFLARED_CONTAINER)
        .await?
        .is_some_and(|container| {
            container.running && container.networks.contains_key(GUMGUM_NETWORK)
        }))
}
