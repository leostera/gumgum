use crate::{
    ConfigStore, ErrorCause, ErrorCode, ErrorKind, GumgumError, Result, Subsystem,
    TokenPromptRequirement,
};
use serde::{Deserialize, Serialize};

use super::types::{CLOUDFLARE_PROVIDER, CloudflareGrant};

const REQUIRED_PERMISSIONS: &[(
    CloudflarePermissionScope,
    CloudflarePermissionGrant,
    CloudflarePermissionTarget,
)] = &[
    (
        CloudflarePermissionScope::Zone,
        CloudflarePermissionGrant::ZoneRead,
        CloudflarePermissionTarget::ManagedZones,
    ),
    (
        CloudflarePermissionScope::Zone,
        CloudflarePermissionGrant::DnsWrite,
        CloudflarePermissionTarget::ManagedZones,
    ),
    (
        CloudflarePermissionScope::Account,
        CloudflarePermissionGrant::CloudflaredWrite,
        CloudflarePermissionTarget::TunnelIngressAccount,
    ),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflarePermissionScope {
    Zone,
    Account,
}

impl CloudflarePermissionScope {
    pub fn cloudflare_name(self) -> &'static str {
        match self {
            CloudflarePermissionScope::Zone => "Zone",
            CloudflarePermissionScope::Account => "Account",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflarePermissionGrant {
    ZoneRead,
    DnsWrite,
    CloudflaredWrite,
}

impl CloudflarePermissionGrant {
    pub fn cloudflare_name(self) -> &'static str {
        match self {
            CloudflarePermissionGrant::ZoneRead => "Zone Read",
            CloudflarePermissionGrant::DnsWrite => "DNS Write",
            CloudflarePermissionGrant::CloudflaredWrite => {
                "Cloudflare One Connector: cloudflared Write"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflarePermissionTarget {
    ManagedZones,
    TunnelIngressAccount,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareTokenPrompt {
    pub zone_name: String,
    pub token_url: String,
    pub permissions: Vec<CloudflareTokenPermission>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareTokenPermission {
    pub scope: CloudflarePermissionScope,
    pub permission: CloudflarePermissionGrant,
    pub applies_to: CloudflarePermissionTarget,
}

pub fn token_prompt(zone_name: &str) -> CloudflareTokenPrompt {
    CloudflareTokenPrompt {
        zone_name: zone_name.to_owned(),
        token_url: "https://dash.cloudflare.com/profile/api-tokens".to_owned(),
        permissions: REQUIRED_PERMISSIONS
            .iter()
            .map(
                |(scope, permission, applies_to)| CloudflareTokenPermission {
                    scope: *scope,
                    permission: *permission,
                    applies_to: *applies_to,
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
        return Err(GumgumError::structured_kind(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            ErrorKind::CloudflareTokenRequired,
        )
        .cause(ErrorCause::CloudflareTokenPrompt {
            zone: zone_name.to_owned(),
            requirement: TokenPromptRequirement::InteractiveRequired,
        })
        .next_command(format!(
            "gumgum domain add {zone_name} --provider cloudflare --ingress cloudflare"
        ))
        .build());
    }
    Err(GumgumError::structured_kind(
        Subsystem::Config,
        ErrorCode::InvalidArgs,
        ErrorKind::CloudflareTokenRequired,
    )
    .cause(ErrorCause::CloudflareTokenPrompt {
        zone: zone_name.to_owned(),
        requirement: TokenPromptRequirement::CallerRequired,
    })
    .next_command(format!(
        "gumgum domain add {zone_name} --provider cloudflare --ingress cloudflare"
    ))
    .build())
}

pub fn grant_from_api_token(zone_name: &str, token: impl Into<String>) -> Result<CloudflareGrant> {
    let token = token.into().trim().to_owned();
    if token.is_empty() {
        return Err(GumgumError::structured_kind(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            ErrorKind::CloudflareTokenEmpty,
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
            .map(|(scope, permission, applies_to)| {
                format!(
                    "{}:{}:{:?}",
                    scope.cloudflare_name(),
                    permission.cloudflare_name(),
                    applies_to
                )
            })
            .collect(),
    })
}

#[allow(dead_code)]
fn _provider_name() -> &'static str {
    CLOUDFLARE_PROVIDER
}
