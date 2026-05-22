use crate::IngressMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DomainProvider {
    Manual,
    Cloudflare,
}

impl DomainProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Cloudflare => "cloudflare",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DomainRecord {
    pub name: String,
    pub provider: DomainProvider,
    pub ingress: IngressMode,
}
