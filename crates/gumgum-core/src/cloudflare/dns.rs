use crate::{CloudflareGrant, DomainProvider, DomainRecord, IngressMode, Result};

use super::api::CloudflareClient;

pub async fn ensure_published_route(
    domains: &[DomainRecord],
    grant: &CloudflareGrant,
    hostname: &str,
) -> Result<Vec<String>> {
    let Some(domain) = matching_domain(domains, hostname) else {
        return Err(crate::GumgumError::structured(
            crate::Subsystem::Config,
            crate::ErrorCode::InvalidArgs,
            format!("no managed domain matches published route {hostname}"),
        )
        .likely_cause("add the domain to this server before deploying a published route")
        .next_command("gumgum domain add <domain> --provider cloudflare --ingress cloudflare")
        .build());
    };
    match (domain.provider, domain.ingress) {
        (DomainProvider::Cloudflare, IngressMode::Cloudflare) => {
            let route = CloudflareClient::new(grant)
                .ensure_route(&domain.name, hostname)
                .await?;
            let mut actions = route.actions;
            actions.append(&mut super::tunnel::ensure_cloudflared(&route.tunnel_token).await?);
            Ok(actions)
        }
        (DomainProvider::Manual, _) => Ok(vec![format!(
            "manual DNS required for {hostname} under {}",
            domain.name
        )]),
        (DomainProvider::Cloudflare, IngressMode::Direct) => Ok(vec![format!(
            "Cloudflare direct DNS for {hostname} is not implemented yet"
        )]),
    }
}

pub async fn delete_published_route(
    domains: &[DomainRecord],
    grant: &CloudflareGrant,
    hostname: &str,
) -> Result<Vec<String>> {
    let Some(domain) = matching_domain(domains, hostname) else {
        return Ok(vec![format!(
            "no managed domain matches stale route {hostname}; DNS was not changed"
        )]);
    };
    match (domain.provider, domain.ingress) {
        (DomainProvider::Cloudflare, _) => {
            CloudflareClient::new(grant)
                .delete_route_dns(&domain.name, hostname)
                .await
        }
        (DomainProvider::Manual, _) => Ok(vec![format!(
            "manual DNS cleanup required for stale route {hostname} under {}",
            domain.name
        )]),
    }
}

fn matching_domain<'a>(domains: &'a [DomainRecord], hostname: &str) -> Option<&'a DomainRecord> {
    domains
        .iter()
        .filter(|domain| {
            hostname == domain.name || hostname.ends_with(&format!(".{}", domain.name))
        })
        .max_by_key(|domain| domain.name.len())
}
