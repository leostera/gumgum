use crate::{ConfigStore, ErrorCode, GumgumError, Result, Subsystem};
use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareTokenPrompt {
    pub zone_name: String,
    pub token_url: String,
    pub permissions: Vec<CloudflareTokenPermission>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareTokenPermission {
    pub scope: String,
    pub permission: String,
    pub applies_to: String,
}

pub fn token_prompt(zone_name: &str) -> CloudflareTokenPrompt {
    CloudflareTokenPrompt {
        zone_name: zone_name.to_owned(),
        token_url: "https://dash.cloudflare.com/profile/api-tokens".to_owned(),
        permissions: REQUIRED_PERMISSIONS
            .iter()
            .map(
                |(scope, permission, applies_to)| CloudflareTokenPermission {
                    scope: (*scope).to_owned(),
                    permission: (*permission).to_owned(),
                    applies_to: (*applies_to).to_owned(),
                },
            )
            .collect(),
    }
}

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
    Err(GumgumError::structured(
        Subsystem::Config,
        ErrorCode::InvalidArgs,
        format!("Cloudflare API token required for {zone_name}"),
    )
    .likely_cause("cloudflare ingress needs a token supplied by the caller")
    .next_command("collect a token using the typed Cloudflare token prompt")
    .build())
}

pub fn grant_from_api_token(zone_name: &str, token: impl Into<String>) -> Result<CloudflareGrant> {
    let token = token.into().trim().to_owned();
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
