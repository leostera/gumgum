use crate::Capability;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSpec {
    pub capability: Capability,
    pub provider: String,
    pub container: String,
    pub image: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObjectProviderPlan {
    pub capability: Capability,
    pub name: String,
    pub dns: String,
    pub provider: ProviderSpec,
    pub actions: Vec<String>,
    pub connection_examples: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderConfig {
    pub capability: Capability,
    pub provider: String,
    pub kind: String,
    pub endpoint: Option<String>,
    pub vault: Option<String>,
}

impl ProviderConfig {
    pub fn new(
        capability: Capability,
        kind: impl Into<String>,
        endpoint: Option<String>,
        vault: Option<String>,
    ) -> Self {
        let kind = kind.into();
        let provider = match (capability, kind.as_str()) {
            (Capability::Secret, "local") => "local.secrets".to_owned(),
            (Capability::Secret, "vaultwarden") | (Capability::Secret, "bitwarden") => {
                "vaultwarden.main".to_owned()
            }
            (Capability::Secret, "onepassword") | (Capability::Secret, "onepassword-connect") => {
                "onepassword.main".to_owned()
            }
            _ => capability.provider().to_owned(),
        };
        Self {
            capability,
            provider,
            kind,
            endpoint,
            vault,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCredentials {
    pub username_env: String,
    pub password_env: String,
    pub username: String,
    pub password: String,
}

impl ProviderCredentials {
    pub fn minio_local_dev() -> Self {
        Self {
            username_env: "MINIO_ROOT_USER".to_owned(),
            password_env: "MINIO_ROOT_PASSWORD".to_owned(),
            username: std::env::var("GUMGUM_MINIO_ROOT_USER")
                .unwrap_or_else(|_| "gumgum".to_owned()),
            password: std::env::var("GUMGUM_MINIO_ROOT_PASSWORD")
                .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
        }
    }

    pub fn minio_generated() -> Self {
        Self::generated("MINIO_ROOT_USER", "MINIO_ROOT_PASSWORD", "gumgum")
    }

    pub fn postgres_local_dev() -> Self {
        Self {
            username_env: "POSTGRES_USER".to_owned(),
            password_env: "POSTGRES_PASSWORD".to_owned(),
            username: std::env::var("GUMGUM_POSTGRES_USER").unwrap_or_else(|_| "gumgum".to_owned()),
            password: std::env::var("GUMGUM_POSTGRES_PASSWORD")
                .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
        }
    }

    pub fn postgres_generated() -> Self {
        Self::generated("POSTGRES_USER", "POSTGRES_PASSWORD", "gumgum")
    }

    pub fn redis_local_dev() -> Self {
        Self {
            username_env: "REDIS_USER".to_owned(),
            password_env: "REDIS_PASSWORD".to_owned(),
            username: std::env::var("GUMGUM_REDIS_USER").unwrap_or_else(|_| "gumgum".to_owned()),
            password: std::env::var("GUMGUM_REDIS_PASSWORD")
                .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
        }
    }

    pub fn redis_generated() -> Self {
        Self::generated("REDIS_USER", "REDIS_PASSWORD", "gumgum")
    }

    pub fn generated(username_env: &str, password_env: &str, username: &str) -> Self {
        Self {
            username_env: username_env.to_owned(),
            password_env: password_env.to_owned(),
            username: username.to_owned(),
            password: generated_secret_value(),
        }
    }
}

pub fn generated_secret_value() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ((seed >> ((index % 16) * 8)) & 0xff) as u8;
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub capability: Capability,
    pub provider: String,
    pub container: String,
    pub image: String,
    pub port: u16,
    pub running: bool,
}
