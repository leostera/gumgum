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
    pub binds: Vec<String>,
    pub ports: Vec<PortBindingSpec>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortBindingSpec {
    pub host_ip: Option<String>,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

impl PortBindingSpec {
    pub fn tcp(host_ip: Option<String>, host_port: u16, container_port: u16) -> Self {
        Self {
            host_ip,
            host_port,
            container_port,
            protocol: "tcp".to_owned(),
        }
    }

    pub fn udp(host_ip: Option<String>, host_port: u16, container_port: u16) -> Self {
        Self {
            host_ip,
            host_port,
            container_port,
            protocol: "udp".to_owned(),
        }
    }

    fn key(&self) -> String {
        format!("{}/{}", self.container_port, self.protocol)
    }
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

    pub async fn ensure_network(&self, name: &str) -> Result<bool> {
        use bollard::models::NetworkCreateRequest;
        if self.network_exists(name).await? {
            return Ok(false);
        }
        self.client
            .create_network(NetworkCreateRequest {
                name: name.to_owned(),
                ..Default::default()
            })
            .await
            .map_err(docker_error)?;
        Ok(true)
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
            ContainerCreateBody, HostConfig, PortBinding, PortMap, RestartPolicy,
            RestartPolicyNameEnum,
        };
        use bollard::query_parameters::{CreateContainerOptionsBuilder, StartContainerOptions};

        let exposed_ports = if spec.ports.is_empty() {
            None
        } else {
            Some(spec.ports.iter().map(PortBindingSpec::key).collect())
        };
        let mut port_bindings: PortMap = HashMap::new();
        for port in &spec.ports {
            port_bindings
                .entry(port.key())
                .or_default()
                .get_or_insert_with(Vec::new)
                .push(PortBinding {
                    host_ip: port.host_ip.clone(),
                    host_port: Some(port.host_port.to_string()),
                });
        }
        let body = ContainerCreateBody {
            image: Some(spec.image),
            env: Some(
                spec.env
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect(),
            ),
            labels: Some(spec.labels),
            exposed_ports,
            host_config: Some(HostConfig {
                network_mode: Some(spec.network),
                binds: Some(spec.binds),
                port_bindings: (!port_bindings.is_empty()).then_some(port_bindings),
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

    pub async fn start_container(&self, name: &str) -> Result<()> {
        use bollard::query_parameters::StartContainerOptions;
        match self
            .client
            .start_container(name, None::<StartContainerOptions>)
            .await
        {
            Ok(()) => Ok(()),
            Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(error) => Err(docker_error(error)),
        }
    }

    pub async fn restart_container(&self, name: &str) -> Result<()> {
        use bollard::query_parameters::RestartContainerOptionsBuilder;
        let options = RestartContainerOptionsBuilder::new().build();
        self.client
            .restart_container(name, Some(options))
            .await
            .map_err(docker_error)
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
