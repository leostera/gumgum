use serde::{Deserialize, Serialize};

pub const CLOUDFLARE_PROVIDER: &str = "cloudflare.main";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareGrant {
    pub account_id: Option<String>,
    pub zone_id: Option<String>,
    pub zone_name: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CloudflareTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IngressMode {
    #[default]
    Direct,
    Cloudflare,
}

impl IngressMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Cloudflare => "cloudflare",
        }
    }
}
