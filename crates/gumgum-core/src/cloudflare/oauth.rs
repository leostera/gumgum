use crate::{ConfigStore, ErrorCode, GumgumError, Result, Subsystem};
use std::io::{self, Write};

use super::types::{CLOUDFLARE_PROVIDER, CloudflareGrant};

const REQUIRED_PERMISSIONS: &[(&str, &str, &str)] = &[
    ("Zone", "Zone Read", "All zones GumGum should manage"),
    ("Zone", "DNS Write", "All zones GumGum should manage"),
    (
        "Account",
        "Cloudflare One Connector: cloudflared Write",
        "Account used for Cloudflare Tunnel ingress",
    ),
];

pub async fn ensure_authorized_for_zone(
    store: &ConfigStore,
    zone_name: &str,
    interactive: bool,
) -> Result<CloudflareGrant> {
    if let Some(grant) = store.load_cloudflare_grant()? {
        if grant.zone_name == zone_name {
            return Ok(grant);
        }
    }
    if !interactive {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!("Cloudflare API token required for {zone_name}"),
        )
        .likely_cause("cloudflare ingress needs an interactive token prompt")
        .next_command("rerun without --json or --dry-run in an interactive terminal")
        .build());
    }
    let grant = authorize_zone(zone_name).await?;
    store.save_cloudflare_grant(&grant)?;
    Ok(grant)
}

pub async fn authorize_zone(zone_name: &str) -> Result<CloudflareGrant> {
    eprintln!("Cloudflare API token required for {zone_name}.");
    eprintln!("Create a Cloudflare API token with these permission policies:");
    eprintln!("  Scope    Permission                                      Applies to");
    eprintln!(
        "  -------  ----------------------------------------------  ---------------------------------------------------------------"
    );
    for (scope, permission, applies_to) in REQUIRED_PERMISSIONS {
        eprintln!("  {scope:<7}  {permission:<46}  {applies_to}");
    }
    eprintln!("Use Cloudflare's token builder as:");
    eprintln!("  All zones in your account: Zone Read, DNS Write");
    eprintln!("  Entire account: Cloudflare One Connector: cloudflared Write");
    eprintln!("Make sure the zone list includes: {zone_name}");
    eprintln!(
        "When adding more Cloudflare-managed domains later, update/recreate the token to include those domains too."
    );
    eprintln!("Cloudflare token page: https://dash.cloudflare.com/profile/api-tokens");
    eprint!("Paste Cloudflare API token: ");
    io::stderr().flush().map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not flush prompt")
            .likely_cause(source.to_string())
            .build()
    })?;
    let mut token = String::new();
    io::stdin().read_line(&mut token).map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not read Cloudflare token",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            "Cloudflare token cannot be empty",
        )
        .build());
    }
    Ok(CloudflareGrant {
        account_id: None,
        zone_id: None,
        zone_name: zone_name.to_owned(),
        access_token: token,
        refresh_token: None,
        expires_in: None,
        scopes: REQUIRED_PERMISSIONS
            .iter()
            .map(|(scope, permission, applies_to)| format!("{scope} / {permission} / {applies_to}"))
            .collect(),
    })
}

#[allow(dead_code)]
fn _provider_name() -> &'static str {
    CLOUDFLARE_PROVIDER
}
