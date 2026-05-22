use crate::{ErrorCode, GumgumError, Result, Subsystem};
use bollard::{Docker, errors::Error as DockerError};
use std::collections::HashMap;

#[derive(Clone)]
pub struct DockerEngine {
    client: Docker,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContainerSnapshot {
    pub name: String,
    pub image: Option<String>,
    pub labels: HashMap<String, String>,
    pub running: bool,
    pub healthy: Option<bool>,
    pub networks: HashMap<String, String>,
}

impl DockerEngine {
    pub fn local() -> Result<Self> {
        let client = Docker::connect_with_local_defaults().map_err(docker_error)?;
        Ok(Self { client })
    }

    pub async fn inspect_container(&self, name: &str) -> Result<Option<ContainerSnapshot>> {
        match self.client.inspect_container(name, None).await {
            Ok(container) => Ok(Some(ContainerSnapshot {
                name: container
                    .name
                    .as_deref()
                    .unwrap_or(name)
                    .trim_start_matches('/')
                    .to_owned(),
                image: container
                    .config
                    .as_ref()
                    .and_then(|config| config.image.clone()),
                labels: container
                    .config
                    .as_ref()
                    .and_then(|config| config.labels.clone())
                    .unwrap_or_default(),
                running: container
                    .state
                    .as_ref()
                    .and_then(|state| state.running)
                    .unwrap_or(false),
                healthy: container.state.as_ref().and_then(|state| {
                    state
                        .health
                        .as_ref()
                        .and_then(|health| health.status.as_ref())
                        .map(|status| status.to_string() == "healthy")
                }),
                networks: container
                    .network_settings
                    .as_ref()
                    .and_then(|settings| settings.networks.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(network, endpoint)| {
                        let ip = endpoint.ip_address.unwrap_or_default();
                        (network, ip)
                    })
                    .collect(),
            })),
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(None),
            Err(error) => Err(docker_error(error)),
        }
    }

    pub async fn container_running(&self, name: &str) -> Result<bool> {
        Ok(self
            .inspect_container(name)
            .await?
            .map(|container| container.running)
            .unwrap_or(false))
    }

    pub async fn network_exists(&self, name: &str) -> Result<bool> {
        match self.client.inspect_network(name, None).await {
            Ok(_) => Ok(true),
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(false),
            Err(error) => Err(docker_error(error)),
        }
    }

    pub async fn remove_container_force(&self, name: &str) -> Result<()> {
        use bollard::query_parameters::RemoveContainerOptionsBuilder;
        let options = RemoveContainerOptionsBuilder::new().force(true).build();
        match self.client.remove_container(name, Some(options)).await {
            Ok(()) => Ok(()),
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            Err(error) => Err(docker_error(error)),
        }
    }

    pub async fn list_container_names_by_label(&self, labels: &[String]) -> Result<Vec<String>> {
        use bollard::query_parameters::ListContainersOptionsBuilder;
        let mut filters = HashMap::new();
        filters.insert("label".to_owned(), labels.to_vec());
        let options = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();
        let containers = self
            .client
            .list_containers(Some(options))
            .await
            .map_err(docker_error)?;
        Ok(containers
            .into_iter()
            .flat_map(|container| container.names.unwrap_or_default())
            .map(|name| name.trim_start_matches('/').to_owned())
            .filter(|name| !name.is_empty())
            .collect())
    }
}

fn docker_error(source: DockerError) -> GumgumError {
    GumgumError::structured(
        Subsystem::Setup,
        ErrorCode::Io,
        "Docker daemon request failed",
    )
    .likely_cause(source.to_string())
    .build()
}
