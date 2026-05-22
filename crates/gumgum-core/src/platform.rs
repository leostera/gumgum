use crate::{ErrorCode, GumgumError, Subsystem};
use tokio::process::Command as TokioCommand;

pub struct LocalPlatform;

impl LocalPlatform {
    pub async fn ensure(quiet: bool) -> crate::Result<()> {
        run_command_streaming(
            TokioCommand::new("sh").arg("-c").arg("docker network inspect gumgum-network >/dev/null 2>&1 || docker network create gumgum-network >/dev/null"),
            quiet,
        )
        .await?;

        Self::ensure_registry(quiet).await?;
        Self::ensure_dnsmasq(quiet).await?;
        Self::ensure_caddy(quiet).await
    }

    async fn ensure_registry(quiet: bool) -> crate::Result<()> {
        run_command_streaming(
            TokioCommand::new("sh").arg("-c").arg("docker inspect gumgum-registry >/dev/null 2>&1 && docker start gumgum-registry >/dev/null || docker run -d --name gumgum-registry --restart unless-stopped --network gumgum-network -p 127.0.0.1:55000:5000 registry:2 >/dev/null"),
            quiet,
        )
        .await
    }

    async fn ensure_dnsmasq(quiet: bool) -> crate::Result<()> {
        let script = "set -e; mkdir -p ~/.gumgum/dnsmasq; host_ip=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i=1;i<=NF;i++) if ($i==\"src\") {print $(i+1); exit}}'); [ -n \"$host_ip\" ] || host_ip=127.0.0.1; upstream=$(resolvectl dns 2>/dev/null | awk -v host_ip=\"$host_ip\" '{for (i=1;i<=NF;i++) if ($i ~ /^[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+$/ && $i != host_ip && $i !~ /^127\\./) {print $i; exit}}'); if [ -z \"$upstream\" ]; then upstream=$(ip route 2>/dev/null | awk '/^default/ {print $3; exit}'); fi; [ -n \"$upstream\" ] || upstream=1.1.1.1; tmp=$(mktemp); { printf 'listen-address=0.0.0.0\nbind-interfaces\nno-resolv\nserver=%s\ncache-size=10000\n' \"$upstream\"; if [ -f ~/.gumgum/dnsmasq/dnsmasq.conf ]; then grep '^address=/' ~/.gumgum/dnsmasq/dnsmasq.conf || true; fi; } > $tmp; mv $tmp ~/.gumgum/dnsmasq/dnsmasq.conf; if docker inspect gumgum-dnsmasq >/dev/null 2>&1; then docker start gumgum-dnsmasq >/dev/null; docker restart gumgum-dnsmasq >/dev/null; elif docker ps --format '{{.Ports}}' | grep -qE \"(^|, )${host_ip}:53->|(^|, )0\\.0\\.0\\.0:53->|(^|, )[^ ]*:53->|:53->\"; then echo \"warning: ${host_ip}:53 is already in use; gumgum-dnsmasq not started\" >&2; else docker run -d --name gumgum-dnsmasq --restart unless-stopped --network gumgum-network -p ${host_ip}:53:53/tcp -p ${host_ip}:53:53/udp -v $HOME/.gumgum/dnsmasq/dnsmasq.conf:/etc/dnsmasq.conf:ro jpillora/dnsmasq:latest >/dev/null; fi";
        run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
    }

    async fn ensure_caddy(quiet: bool) -> crate::Result<()> {
        let script = "set -e; if docker inspect gumgum-caddy >/dev/null 2>&1; then docker start gumgum-caddy >/dev/null; elif docker ps --format '{{.Ports}}' | grep -qE '(^|, )0\\.0\\.0\\.0:(80|443)->|(^|, )[^ ]*:(80|443)->'; then echo 'warning: ports 80/443 are already in use; starting gumgum-caddy for tunnel-only ingress without host port bindings' >&2; docker run -d --name gumgum-caddy --restart unless-stopped --network gumgum-network -v /var/run/docker.sock:/var/run/docker.sock:ro lucaslorentz/caddy-docker-proxy:2.9-alpine >/dev/null; else docker run -d --name gumgum-caddy --restart unless-stopped --network gumgum-network -p 80:80 -p 443:443 -v /var/run/docker.sock:/var/run/docker.sock:ro lucaslorentz/caddy-docker-proxy:2.9-alpine >/dev/null; fi";
        run_command_streaming(TokioCommand::new("sh").arg("-c").arg(script), quiet).await
    }
}

async fn run_command_streaming(cmd: &mut TokioCommand, quiet: bool) -> crate::Result<()> {
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
