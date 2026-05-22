use crate::{ContainerRunSpec, DockerEngine, ErrorCode, GumgumError, PortBindingSpec, Subsystem};
use std::collections::HashMap;
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    path::PathBuf,
    time::Instant,
};
use tokio::process::Command as TokioCommand;

const GUMGUM_NETWORK: &str = "gumgum-network";
const REGISTRY_CONTAINER: &str = "gumgum-registry";
const DNSMASQ_CONTAINER: &str = "gumgum-dnsmasq";
const CADDY_CONTAINER: &str = "gumgum-caddy";

pub struct LocalPlatform;

impl LocalPlatform {
    pub async fn ensure(quiet: bool) -> crate::Result<()> {
        platform_step(quiet, "checking Docker network gumgum-network", async {
            ensure_network(GUMGUM_NETWORK, quiet).await
        })
        .await?;
        platform_step(quiet, "checking local registry container", async {
            Self::ensure_registry(quiet).await
        })
        .await?;
        platform_step(quiet, "checking DNS forwarding container", async {
            Self::ensure_dnsmasq(quiet).await
        })
        .await?;
        platform_step(quiet, "checking HTTP gateway container", async {
            Self::ensure_caddy(quiet).await
        })
        .await
    }

    async fn ensure_registry(quiet: bool) -> crate::Result<()> {
        let docker = DockerEngine::local()?;
        if docker
            .inspect_container(REGISTRY_CONTAINER)
            .await?
            .is_some()
        {
            docker.start_container(REGISTRY_CONTAINER).await?;
            return Ok(());
        }
        if !quiet {
            eprintln!("  create container {REGISTRY_CONTAINER}");
        }
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
            })
            .await
    }

    async fn ensure_dnsmasq(quiet: bool) -> crate::Result<()> {
        let host_ip = host_lan_ip().await.unwrap_or(Ipv4Addr::LOCALHOST);
        let upstream = upstream_dns(host_ip)
            .await
            .unwrap_or(Ipv4Addr::new(1, 1, 1, 1));
        if !quiet {
            eprintln!("  DNS bind address: {host_ip}; upstream: {upstream}");
        }
        write_dnsmasq_config(upstream)?;

        let docker = DockerEngine::local()?;
        if docker.inspect_container(DNSMASQ_CONTAINER).await?.is_some() {
            docker.start_container(DNSMASQ_CONTAINER).await?;
            docker.restart_container(DNSMASQ_CONTAINER).await?;
            return Ok(());
        }

        if !port_available(host_ip, 53) {
            if !quiet {
                eprintln!(
                    "warning: {host_ip}:53 is already in use; {DNSMASQ_CONTAINER} not started"
                );
            }
            return Ok(());
        }

        let config_mount = format!("{}:/etc/dnsmasq.conf:ro", dnsmasq_config_path()?.display());
        if !quiet {
            eprintln!("  create container {DNSMASQ_CONTAINER}");
        }
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
            })
            .await
    }

    async fn ensure_caddy(quiet: bool) -> crate::Result<()> {
        let docker = DockerEngine::local()?;
        if docker.inspect_container(CADDY_CONTAINER).await?.is_some() {
            docker.start_container(CADDY_CONTAINER).await?;
            return Ok(());
        }

        let expose_host_ports =
            port_available(Ipv4Addr::UNSPECIFIED, 80) && port_available(Ipv4Addr::UNSPECIFIED, 443);
        if !expose_host_ports && !quiet {
            eprintln!(
                "warning: ports 80/443 are already in use; starting {CADDY_CONTAINER} for tunnel-only ingress without host port bindings"
            );
        }

        let socket_mount = "/var/run/docker.sock:/var/run/docker.sock:ro";
        let ports = if expose_host_ports {
            vec![
                PortBindingSpec::tcp(None, 80, 80),
                PortBindingSpec::tcp(None, 443, 443),
            ]
        } else {
            Vec::new()
        };
        if !quiet {
            eprintln!("  create container {CADDY_CONTAINER}");
        }
        docker
            .pull_image("lucaslorentz/caddy-docker-proxy:2.9-alpine")
            .await?;
        docker
            .create_and_start_container(ContainerRunSpec {
                name: CADDY_CONTAINER.to_owned(),
                image: "lucaslorentz/caddy-docker-proxy:2.9-alpine".to_owned(),
                network: GUMGUM_NETWORK.to_owned(),
                restart_unless_stopped: true,
                labels: HashMap::new(),
                env: Vec::new(),
                binds: vec![socket_mount.to_owned()],
                ports,
                command: Vec::new(),
            })
            .await
    }
}

async fn platform_step<T>(
    quiet: bool,
    label: impl AsRef<str>,
    future: impl Future<Output = crate::Result<T>>,
) -> crate::Result<T> {
    let label = label.as_ref();
    if !quiet {
        eprintln!("→ {label}");
    }
    let started = Instant::now();
    let result = future.await;
    if !quiet {
        match &result {
            Ok(_) => eprintln!("✓ {label} ({:.1}s)", started.elapsed().as_secs_f32()),
            Err(_) => eprintln!("✗ {label} ({:.1}s)", started.elapsed().as_secs_f32()),
        }
    }
    result
}

async fn ensure_network(name: &str, quiet: bool) -> crate::Result<()> {
    let docker = DockerEngine::local()?;
    let created = docker.ensure_network(name).await?;
    if created && !quiet {
        eprintln!("  created Docker network {name}");
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
