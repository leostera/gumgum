use crate::{ErrorCode, GumgumError, Subsystem};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket},
    path::PathBuf,
};
use tokio::process::Command as TokioCommand;

const GUMGUM_NETWORK: &str = "gumgum-network";
const REGISTRY_CONTAINER: &str = "gumgum-registry";
const DNSMASQ_CONTAINER: &str = "gumgum-dnsmasq";
const CADDY_CONTAINER: &str = "gumgum-caddy";

pub struct LocalPlatform;

impl LocalPlatform {
    pub async fn ensure(quiet: bool) -> crate::Result<()> {
        ensure_network(GUMGUM_NETWORK, quiet).await?;
        Self::ensure_registry(quiet).await?;
        Self::ensure_dnsmasq(quiet).await?;
        Self::ensure_caddy(quiet).await
    }

    async fn ensure_registry(quiet: bool) -> crate::Result<()> {
        if container_exists(REGISTRY_CONTAINER).await? {
            docker(["start", REGISTRY_CONTAINER], quiet).await?;
            return Ok(());
        }
        docker(
            [
                "run",
                "-d",
                "--name",
                REGISTRY_CONTAINER,
                "--restart",
                "unless-stopped",
                "--network",
                GUMGUM_NETWORK,
                "-p",
                "127.0.0.1:55000:5000",
                "registry:2",
            ],
            quiet,
        )
        .await
    }

    async fn ensure_dnsmasq(quiet: bool) -> crate::Result<()> {
        let host_ip = host_lan_ip().await.unwrap_or(Ipv4Addr::LOCALHOST);
        let upstream = upstream_dns(host_ip)
            .await
            .unwrap_or(Ipv4Addr::new(1, 1, 1, 1));
        write_dnsmasq_config(upstream)?;

        if container_exists(DNSMASQ_CONTAINER).await? {
            docker(["start", DNSMASQ_CONTAINER], quiet).await?;
            docker(["restart", DNSMASQ_CONTAINER], quiet).await?;
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
        let tcp_port = format!("{host_ip}:53:53/tcp");
        let udp_port = format!("{host_ip}:53:53/udp");
        docker(
            [
                "run",
                "-d",
                "--name",
                DNSMASQ_CONTAINER,
                "--restart",
                "unless-stopped",
                "--network",
                GUMGUM_NETWORK,
                "-p",
                tcp_port.as_str(),
                "-p",
                udp_port.as_str(),
                "-v",
                config_mount.as_str(),
                "jpillora/dnsmasq:latest",
            ],
            quiet,
        )
        .await
    }

    async fn ensure_caddy(quiet: bool) -> crate::Result<()> {
        if container_exists(CADDY_CONTAINER).await? {
            docker(["start", CADDY_CONTAINER], quiet).await?;
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
        if expose_host_ports {
            docker(
                [
                    "run",
                    "-d",
                    "--name",
                    CADDY_CONTAINER,
                    "--restart",
                    "unless-stopped",
                    "--network",
                    GUMGUM_NETWORK,
                    "-p",
                    "80:80",
                    "-p",
                    "443:443",
                    "-v",
                    socket_mount,
                    "lucaslorentz/caddy-docker-proxy:2.9-alpine",
                ],
                quiet,
            )
            .await
        } else {
            docker(
                [
                    "run",
                    "-d",
                    "--name",
                    CADDY_CONTAINER,
                    "--restart",
                    "unless-stopped",
                    "--network",
                    GUMGUM_NETWORK,
                    "-v",
                    socket_mount,
                    "lucaslorentz/caddy-docker-proxy:2.9-alpine",
                ],
                quiet,
            )
            .await
        }
    }
}

async fn ensure_network(name: &str, quiet: bool) -> crate::Result<()> {
    if command_success(
        TokioCommand::new("docker")
            .arg("network")
            .arg("inspect")
            .arg(name),
    )
    .await?
    {
        return Ok(());
    }
    docker(["network", "create", name], quiet).await
}

async fn container_exists(name: &str) -> crate::Result<bool> {
    command_success(TokioCommand::new("docker").arg("inspect").arg(name)).await
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

async fn docker<'a, I>(args: I, quiet: bool) -> crate::Result<()>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut command = TokioCommand::new("docker");
    command.args(args);
    run_command(&mut command, quiet).await
}

async fn command_success(command: &mut TokioCommand) -> crate::Result<bool> {
    let output = command.output().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "failed to run platform command",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    Ok(output.status.success())
}

async fn run_command(cmd: &mut TokioCommand, quiet: bool) -> crate::Result<()> {
    let output = cmd.output().await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "failed to run platform command",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    if !quiet {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "platform command failed")
                .likely_cause(format!("process exited with {}", output.status))
                .build(),
        )
    }
}
