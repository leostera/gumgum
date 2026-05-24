use crate::{ContainerRunSpec, DockerEngine, ErrorCode, GumgumError, PortBindingSpec, Subsystem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    path::PathBuf,
    pin::Pin,
    time::Instant,
};
use tokio::process::Command as TokioCommand;

const GUMGUM_NETWORK: &str = "gumgum-network";
const REGISTRY_CONTAINER: &str = "gumgum-registry";
const DNSMASQ_CONTAINER: &str = "gumgum-dnsmasq";
const CADDY_CONTAINER: &str = "gumgum-caddy";

pub struct LocalPlatform;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformEvent {
    StepStarted {
        step: PlatformStep,
    },
    StepFinished {
        step: PlatformStep,
        elapsed_ms: u128,
    },
    StepFailed {
        step: PlatformStep,
        elapsed_ms: u128,
    },
    ContainerCreate {
        container: String,
    },
    ContainerRecreate {
        container: String,
    },
    NetworkCreated {
        network: String,
    },
    DnsConfig {
        bind_address: String,
        upstream: String,
    },
    PortUnavailable {
        bind_address: String,
        port: u16,
        container: String,
    },
    GatewayPortsUnavailable {
        container: String,
        subsystem: Subsystem,
        code: ErrorCode,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformStep {
    DockerNetwork,
    LocalRegistry,
    DnsForwarder,
    HttpGateway,
}

impl LocalPlatform {
    pub async fn ensure(_quiet: bool) -> crate::Result<()> {
        Self::ensure_with_events(|_| {}).await
    }

    pub async fn ensure_with_events(mut emit: impl FnMut(PlatformEvent)) -> crate::Result<()> {
        platform_step(&mut emit, PlatformStep::DockerNetwork, |emit| {
            Box::pin(ensure_network(GUMGUM_NETWORK, emit))
        })
        .await?;
        platform_step(&mut emit, PlatformStep::LocalRegistry, |emit| {
            Box::pin(Self::ensure_registry(emit))
        })
        .await?;
        platform_step(&mut emit, PlatformStep::DnsForwarder, |emit| {
            Box::pin(Self::ensure_dnsmasq(emit))
        })
        .await?;
        platform_step(&mut emit, PlatformStep::HttpGateway, |emit| {
            Box::pin(Self::ensure_caddy(emit))
        })
        .await
    }

    async fn ensure_registry(emit: &mut impl FnMut(PlatformEvent)) -> crate::Result<()> {
        let docker = DockerEngine::local()?;
        if docker
            .inspect_container(REGISTRY_CONTAINER)
            .await?
            .is_some()
        {
            docker.start_container(REGISTRY_CONTAINER).await?;
            return Ok(());
        }
        emit(PlatformEvent::ContainerCreate {
            container: REGISTRY_CONTAINER.to_owned(),
        });
        docker.pull_image("registry:2").await?;
        docker
            .create_and_start_container(ContainerRunSpec {
                name: REGISTRY_CONTAINER.to_owned(),
                image: "registry:2".to_owned(),
                network: GUMGUM_NETWORK.to_owned(),
                restart_unless_stopped: true,
                labels: HashMap::new(),
                env: Vec::new(),
                binds: Vec::new(),
                ports: vec![PortBindingSpec::tcp(
                    Some("127.0.0.1".to_owned()),
                    55000,
                    5000,
                )],
                command: Vec::new(),
                entrypoint: Vec::new(),
            })
            .await
    }

    async fn ensure_dnsmasq(emit: &mut impl FnMut(PlatformEvent)) -> crate::Result<()> {
        let host_ip = host_lan_ip().await.unwrap_or(Ipv4Addr::LOCALHOST);
        let upstream = upstream_dns(host_ip)
            .await
            .unwrap_or(Ipv4Addr::new(1, 1, 1, 1));
        emit(PlatformEvent::DnsConfig {
            bind_address: host_ip.to_string(),
            upstream: upstream.to_string(),
        });
        write_dnsmasq_config(upstream)?;

        let docker = DockerEngine::local()?;
        if docker.inspect_container(DNSMASQ_CONTAINER).await?.is_some() {
            docker.start_container(DNSMASQ_CONTAINER).await?;
            docker.restart_container(DNSMASQ_CONTAINER).await?;
            return Ok(());
        }

        if !port_available(host_ip, 53) {
            emit(PlatformEvent::PortUnavailable {
                bind_address: host_ip.to_string(),
                port: 53,
                container: DNSMASQ_CONTAINER.to_owned(),
            });
            return Ok(());
        }

        let config_mount = format!("{}:/etc/dnsmasq.conf:ro", dnsmasq_config_path()?.display());
        emit(PlatformEvent::ContainerCreate {
            container: DNSMASQ_CONTAINER.to_owned(),
        });
        docker.pull_image("jpillora/dnsmasq:latest").await?;
        docker
            .create_and_start_container(ContainerRunSpec {
                name: DNSMASQ_CONTAINER.to_owned(),
                image: "jpillora/dnsmasq:latest".to_owned(),
                network: GUMGUM_NETWORK.to_owned(),
                restart_unless_stopped: true,
                labels: HashMap::new(),
                env: Vec::new(),
                binds: vec![config_mount],
                ports: vec![
                    PortBindingSpec::tcp(Some(host_ip.to_string()), 53, 53),
                    PortBindingSpec::udp(Some(host_ip.to_string()), 53, 53),
                ],
                command: Vec::new(),
                entrypoint: Vec::new(),
            })
            .await
    }

    async fn ensure_caddy(emit: &mut impl FnMut(PlatformEvent)) -> crate::Result<()> {
        let docker = DockerEngine::local()?;
        if let Some(existing) = docker.inspect_container(CADDY_CONTAINER).await? {
            let has_http = existing
                .ports
                .iter()
                .any(|port| port.host_port == 80 && port.container_port == 80);
            let has_https = existing
                .ports
                .iter()
                .any(|port| port.host_port == 443 && port.container_port == 443);
            let has_volume_fingerprint = existing
                .labels
                .get("gumgum.platform.fingerprint")
                .map(String::as_str)
                == Some("caddy-v5");
            if has_http && has_https && has_volume_fingerprint {
                docker.start_container(CADDY_CONTAINER).await?;
                return Ok(());
            }
            emit(PlatformEvent::ContainerRecreate {
                container: CADDY_CONTAINER.to_owned(),
            });
            docker.remove_container_force(CADDY_CONTAINER).await?;
        }

        let socket_mount = "/var/run/docker.sock:/var/run/docker.sock:ro";
        let ports = vec![
            PortBindingSpec::tcp(None, 80, 80),
            PortBindingSpec::tcp(None, 443, 443),
        ];
        emit(PlatformEvent::ContainerCreate {
            container: CADDY_CONTAINER.to_owned(),
        });
        docker
            .pull_image("lucaslorentz/caddy-docker-proxy:latest")
            .await?;
        let spec = |ports| ContainerRunSpec {
            name: CADDY_CONTAINER.to_owned(),
            image: "lucaslorentz/caddy-docker-proxy:latest".to_owned(),
            network: GUMGUM_NETWORK.to_owned(),
            restart_unless_stopped: true,
            labels: HashMap::from([(
                "gumgum.platform.fingerprint".to_owned(),
                "caddy-v5".to_owned(),
            )]),
            binds: vec![
                socket_mount.to_owned(),
                "/gumgum/volumes/platform/caddy-data:/data".to_owned(),
                "/gumgum/volumes/platform/caddy-config:/config".to_owned(),
            ],
            env: vec![
                ("OTEL_SERVICE_NAME".to_owned(), "gumgum-caddy".to_owned()),
                (
                    "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
                    "http://gumgum-otel:4318".to_owned(),
                ),
                (
                    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT".to_owned(),
                    "http://gumgum-otel:4318/v1/traces".to_owned(),
                ),
                (
                    "OTEL_EXPORTER_OTLP_PROTOCOL".to_owned(),
                    "http/protobuf".to_owned(),
                ),
                ("OTEL_TRACES_EXPORTER".to_owned(), "otlp".to_owned()),
                ("OTEL_METRICS_EXPORTER".to_owned(), "none".to_owned()),
                ("OTEL_LOGS_EXPORTER".to_owned(), "none".to_owned()),
                (
                    "OTEL_RESOURCE_ATTRIBUTES".to_owned(),
                    "gumgum.managed=platform,gumgum.platform.service=caddy".to_owned(),
                ),
            ],
            ports,
            command: Vec::new(),
            entrypoint: Vec::new(),
        };
        match docker.create_and_start_container(spec(ports)).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = docker.remove_container_force(CADDY_CONTAINER).await;
                let report = error.to_report();
                emit(PlatformEvent::GatewayPortsUnavailable {
                    container: CADDY_CONTAINER.to_owned(),
                    subsystem: report.subsystem,
                    code: report.code,
                });
                docker.create_and_start_container(spec(Vec::new())).await
            }
        }
    }
}

async fn platform_step<T, E, F>(emit: &mut E, step: PlatformStep, run: F) -> crate::Result<T>
where
    E: FnMut(PlatformEvent),
    F: for<'a> FnOnce(&'a mut E) -> Pin<Box<dyn Future<Output = crate::Result<T>> + 'a>>,
{
    emit(PlatformEvent::StepStarted { step });
    let started = Instant::now();
    let result = run(emit).await;
    let elapsed_ms = started.elapsed().as_millis();
    match &result {
        Ok(_) => emit(PlatformEvent::StepFinished { step, elapsed_ms }),
        Err(_) => emit(PlatformEvent::StepFailed { step, elapsed_ms }),
    }
    result
}

async fn ensure_network(name: &str, emit: &mut impl FnMut(PlatformEvent)) -> crate::Result<()> {
    let docker = DockerEngine::local()?;
    let created = docker.ensure_network(name).await?;
    if created {
        emit(PlatformEvent::NetworkCreated {
            network: name.to_owned(),
        });
    }
    Ok(())
}

async fn host_lan_ip() -> Option<Ipv4Addr> {
    let output = TokioCommand::new("ip")
        .args(["-4", "route", "get", "1.1.1.1"])
        .output()
        .await
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tokens = stdout.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "src" {
            return tokens.next()?.parse().ok();
        }
    }
    None
}

async fn upstream_dns(host_ip: Ipv4Addr) -> Option<Ipv4Addr> {
    let output = TokioCommand::new("resolvectl")
        .arg("dns")
        .output()
        .await
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .filter_map(|token| token.parse::<Ipv4Addr>().ok())
        .find(|ip| *ip != host_ip && !ip.is_loopback())
        .or(Some(Ipv4Addr::new(1, 1, 1, 1)))
}

fn write_dnsmasq_config(upstream: Ipv4Addr) -> crate::Result<()> {
    let path = dnsmasq_config_path()?;
    let mut static_addresses = Vec::new();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        static_addresses.extend(
            existing
                .lines()
                .filter(|line| line.starts_with("address=/"))
                .map(ToOwned::to_owned),
        );
    }

    let mut config = format!(
        "listen-address=0.0.0.0\nbind-interfaces\nno-resolv\nserver={upstream}\ncache-size=10000\n"
    );
    for line in static_addresses {
        config.push_str(&line);
        config.push('\n');
    }
    std::fs::write(path, config).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not write dnsmasq config",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

fn dnsmasq_config_path() -> crate::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not read HOME")
            .likely_cause(source.to_string())
            .build()
    })?;
    let dir = PathBuf::from(home).join(".gumgum").join("dnsmasq");
    std::fs::create_dir_all(&dir).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not create dnsmasq config directory",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    Ok(dir.join("dnsmasq.conf"))
}

fn port_available(host: Ipv4Addr, port: u16) -> bool {
    let addr = SocketAddr::new(IpAddr::V4(host), port);
    TcpListener::bind(addr).is_ok() && UdpSocket::bind(addr).is_ok()
}
