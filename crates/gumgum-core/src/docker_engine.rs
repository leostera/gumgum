use crate::{ErrorCode, GumgumError, Result, Subsystem};
use bollard::{Docker, errors::Error as DockerError};
use futures_util::StreamExt;
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContainerRunSpec {
    pub name: String,
    pub image: String,
    pub network: String,
    pub restart_unless_stopped: bool,
    pub labels: HashMap<String, String>,
    pub env: Vec<(String, String)>,
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

    pub async fn pull_image(&self, image: &str) -> Result<()> {
        use bollard::query_parameters::CreateImageOptionsBuilder;
        let options = CreateImageOptionsBuilder::new().from_image(image).build();
        let mut stream = self.client.create_image(Some(options), None, None);
        while let Some(message) = stream.next().await {
            message.map_err(docker_error)?;
        }
        Ok(())
    }

    pub async fn create_and_start_container(&self, spec: ContainerRunSpec) -> Result<()> {
        use bollard::models::{
            ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum,
        };
        use bollard::query_parameters::{CreateContainerOptionsBuilder, StartContainerOptions};

        let body = ContainerCreateBody {
            image: Some(spec.image),
            env: Some(
                spec.env
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect(),
            ),
            labels: Some(spec.labels),
            host_config: Some(HostConfig {
                network_mode: Some(spec.network),
                restart_policy: spec.restart_unless_stopped.then_some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new()
            .name(&spec.name)
            .build();
        self.client
            .create_container(Some(options), body)
            .await
            .map_err(docker_error)?;
        self.client
            .start_container(&spec.name, None::<StartContainerOptions>)
            .await
            .map_err(docker_error)
    }

    pub async fn connect_container_to_network(&self, container: &str, network: &str) -> Result<()> {
        use bollard::models::NetworkConnectRequest;
        match self
            .client
            .connect_network(
                network,
                NetworkConnectRequest {
                    container: container.to_owned(),
                    endpoint_config: None,
                },
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(DockerError::DockerResponseServerError {
                status_code: 403,
                message,
            }) if message.contains("already exists") || message.contains("already connected") => {
                Ok(())
            }
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
