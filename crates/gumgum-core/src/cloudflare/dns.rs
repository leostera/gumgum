use crate::{
    CloudflareGrant, CoreAction, CoreActions, DomainProvider, DomainRecord, ErrorCause, ErrorCode,
    ErrorKind, IngressMode, Result, Subsystem,
};

use super::api::CloudflareClient;

pub async fn ensure_published_route(
    domains: &[DomainRecord],
    grant: &CloudflareGrant,
    hostname: &str,
) -> Result<CoreActions> {
    let Some(domain) = matching_domain(domains, hostname) else {
        return Err(crate::GumgumError::structured_kind(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            ErrorKind::PublishedRouteDomainNotManaged,
        )
        .cause(ErrorCause::PublishedRouteDomainMissing {
            hostname: hostname.to_owned(),
        })
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
        (DomainProvider::Manual, _) => Ok(vec![CoreAction::ManualDnsRequired {
            hostname: hostname.to_owned(),
            domain: domain.name.clone(),
        }]),
        (DomainProvider::Cloudflare, IngressMode::Direct) => {
            Ok(vec![CoreAction::CloudflareDirectDnsUnsupported {
                hostname: hostname.to_owned(),
            }])
        }
    }
}

pub async fn delete_published_route(
    domains: &[DomainRecord],
    grant: &CloudflareGrant,
    hostname: &str,
) -> Result<CoreActions> {
    let Some(domain) = matching_domain(domains, hostname) else {
        return Ok(vec![CoreAction::NoManagedDomainForStaleRoute {
            hostname: hostname.to_owned(),
        }]);
    };
    match (domain.provider, domain.ingress) {
        (DomainProvider::Cloudflare, _) => {
            CloudflareClient::new(grant)
                .delete_route_dns(&domain.name, hostname)
                .await
        }
        (DomainProvider::Manual, _) => Ok(vec![CoreAction::ManualDnsCleanupRequired {
            hostname: hostname.to_owned(),
            domain: domain.name.clone(),
        }]),
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
