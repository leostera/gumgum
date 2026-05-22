use crate::{
    ErrorCode, GumgumError, ProviderConfig, ProviderCredentials, Result, Subsystem, sanitize_name,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerRecord {
    pub name: String,
    pub host: String,
    pub root_domain: String,
    pub test_domain: String,
    pub health_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigScope {
    Local,
    Server(String),
}

impl ConfigScope {
    pub fn label(&self) -> String {
        match self {
            ConfigScope::Local => "local".to_owned(),
            ConfigScope::Server(name) => format!("server:{name}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    pub fn from_home_env() -> Result<Self> {
        let home = std::env::var("HOME").map_err(|source| {
            GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not read HOME")
                .likely_cause(source.to_string())
                .build()
        })?;
        Ok(Self::new(PathBuf::from(home).join(".gumgum")))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn load_config(&self, scope: &ConfigScope) -> Result<Map<String, Value>> {
        match scope {
            ConfigScope::Local => self.load_local_config(),
            ConfigScope::Server(name) => self.load_server_config(name),
        }
    }

    pub fn save_config(&self, scope: &ConfigScope, values: &Map<String, Value>) -> Result<()> {
        match scope {
            ConfigScope::Local => self.save_local_config(values),
            ConfigScope::Server(name) => self.save_server_config(name, values),
        }
    }

    pub fn load_local_config(&self) -> Result<Map<String, Value>> {
        self.load_config_map(self.local_config_path())
    }

    pub fn save_local_config(&self, values: &Map<String, Value>) -> Result<()> {
        self.save_config_map(self.local_config_path(), values)
    }

    pub fn load_server_config(&self, name: &str) -> Result<Map<String, Value>> {
        self.load_config_map(self.server_config_path(name))
    }

    pub fn save_server_config(&self, name: &str, values: &Map<String, Value>) -> Result<()> {
        self.save_config_map(self.server_config_path(name), values)
    }

    pub fn load_servers(&self) -> Result<Vec<ServerRecord>> {
        let path = self.servers_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not read server list",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        serde_json::from_str(&raw).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not parse server list",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    pub fn load_default_server(&self) -> Result<Option<ServerRecord>> {
        Ok(self.load_servers()?.into_iter().next())
    }

    pub fn load_provider_config(&self, provider: &str) -> Result<Option<ProviderConfig>> {
        let path = self.provider_config_path(provider);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not read provider config",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        serde_json::from_str(&raw).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not parse provider config",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    pub fn save_provider_config(&self, config: &ProviderConfig) -> Result<()> {
        let path = self.provider_config_path(&config.provider);
        self.ensure_parent(&path)?;
        fs::write(
            &path,
            serde_json::to_string_pretty(config).expect("serialize provider config"),
        )
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write provider config",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    pub fn load_provider_credentials(&self, provider: &str) -> Result<Option<ProviderCredentials>> {
        let path = self.provider_credentials_path(provider);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not read provider credentials",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        serde_json::from_str(&raw).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not parse provider credentials",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    pub fn save_provider_credentials(
        &self,
        provider: &str,
        credentials: &ProviderCredentials,
    ) -> Result<()> {
        let path = self.provider_credentials_path(provider);
        self.ensure_parent(&path)?;
        fs::write(
            &path,
            serde_json::to_string_pretty(credentials).expect("serialize provider credentials"),
        )
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write provider credentials",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    pub fn load_or_init_provider_credentials(
        &self,
        provider: &str,
        generate: impl FnOnce() -> ProviderCredentials,
    ) -> Result<ProviderCredentials> {
        if let Some(credentials) = self.load_provider_credentials(provider)? {
            return Ok(credentials);
        }
        let credentials = generate();
        self.save_provider_credentials(provider, &credentials)?;
        Ok(credentials)
    }

    pub fn load_minio_credentials(&self) -> Result<Option<ProviderCredentials>> {
        self.load_provider_credentials("minio.main")
    }

    pub fn save_minio_credentials(&self, credentials: &ProviderCredentials) -> Result<()> {
        self.save_provider_credentials("minio.main", credentials)
    }

    pub fn load_or_init_minio_credentials(&self) -> Result<ProviderCredentials> {
        self.load_or_init_provider_credentials("minio.main", ProviderCredentials::minio_generated)
    }

    pub fn load_or_init_default_provider_credentials(
        &self,
    ) -> Result<Vec<(String, ProviderCredentials)>> {
        Ok(vec![
            (
                "postgres.main".to_owned(),
                self.load_or_init_provider_credentials(
                    "postgres.main",
                    ProviderCredentials::postgres_generated,
                )?,
            ),
            (
                "redis.main".to_owned(),
                self.load_or_init_provider_credentials(
                    "redis.main",
                    ProviderCredentials::redis_generated,
                )?,
            ),
            (
                "minio.main".to_owned(),
                self.load_or_init_minio_credentials()?,
            ),
        ])
    }

    pub fn save_server(&self, server: ServerRecord) -> Result<()> {
        let path = self.servers_path();
        self.ensure_parent(&path)?;
        let mut servers = self.load_servers()?;
        servers.retain(|existing| existing.host != server.host);
        servers.insert(0, server);
        let raw = serde_json::to_string_pretty(&servers).expect("serialize servers");
        fs::write(&path, raw).map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not write server list",
            )
            .likely_cause(source.to_string())
            .build()
        })
    }

    fn servers_path(&self) -> PathBuf {
        self.root.join("servers.json")
    }

    fn local_config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    fn server_config_path(&self, name: &str) -> PathBuf {
        self.root
            .join("servers")
            .join(sanitize_name(name))
            .join("config.json")
    }

    fn provider_credentials_path(&self, provider: &str) -> PathBuf {
        self.root
            .join("providers")
            .join(sanitize_name(provider))
            .join("credentials.json")
    }

    fn provider_config_path(&self, provider: &str) -> PathBuf {
        self.root
            .join("providers")
            .join(sanitize_name(provider))
            .join("config.json")
    }

    fn load_config_map(&self, path: PathBuf) -> Result<Map<String, Value>> {
        if !path.exists() {
            return Ok(Map::new());
        }
        let raw = fs::read_to_string(&path).map_err(|source| {
            GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not read config")
                .likely_cause(source.to_string())
                .build()
        })?;
        serde_json::from_str(&raw).map_err(|source| {
            GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not parse config")
                .likely_cause(source.to_string())
                .build()
        })
    }

    fn save_config_map(&self, path: PathBuf, values: &Map<String, Value>) -> Result<()> {
        self.ensure_parent(&path)?;
        fs::write(
            &path,
            serde_json::to_string_pretty(values).expect("serialize config"),
        )
        .map_err(|source| {
            GumgumError::structured(Subsystem::Config, ErrorCode::Io, "could not write config")
                .likely_cause(source.to_string())
                .build()
        })
    }

    fn ensure_parent(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                GumgumError::structured(
                    Subsystem::Config,
                    ErrorCode::Io,
                    "could not create config directory",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> ConfigStore {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        ConfigStore::new(std::env::temp_dir().join(format!("gumgum-{name}-{nonce}")))
    }

    #[test]
    fn saves_latest_server_first_and_replaces_by_host() {
        let store = temp_store("servers");
        store
            .save_server(ServerRecord {
                name: "first".to_owned(),
                host: "192.168.0.3".to_owned(),
                root_domain: "leostera.dev".to_owned(),
                test_domain: "leostera.test".to_owned(),
                health_url: "http://192.168.0.3:7777/healthz".to_owned(),
            })
            .unwrap();
        store
            .save_server(ServerRecord {
                name: "renamed".to_owned(),
                host: "192.168.0.3".to_owned(),
                root_domain: "example.com".to_owned(),
                test_domain: "example.test".to_owned(),
                health_url: "http://192.168.0.3:7777/healthz".to_owned(),
            })
            .unwrap();
        let servers = store.load_servers().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "renamed");
        assert_eq!(
            store.load_default_server().unwrap().unwrap().root_domain,
            "example.com"
        );
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn config_scope_labels_are_stable() {
        assert_eq!(ConfigScope::Local.label(), "local");
        assert_eq!(
            ConfigScope::Server("starbase".to_owned()).label(),
            "server:starbase"
        );
    }

    #[test]
    fn stores_provider_config_separately_from_credentials() {
        let store = temp_store("provider-config");
        let config = ProviderConfig::new(
            crate::Capability::Secret,
            "onepassword",
            Some("http://onepassword:8080".to_owned()),
            Some("GumGum".to_owned()),
        );

        store.save_provider_config(&config).unwrap();
        assert_eq!(
            store.load_provider_config("onepassword.main").unwrap(),
            Some(config)
        );
        assert!(
            store
                .root()
                .join("providers/onepassword-main/config.json")
                .exists()
        );
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn stores_provider_credentials_separately_from_config() {
        let store = temp_store("provider-credentials");
        let credentials = ProviderCredentials {
            username_env: "MINIO_ROOT_USER".to_owned(),
            password_env: "MINIO_ROOT_PASSWORD".to_owned(),
            username: "gumgum".to_owned(),
            password: "secret".to_owned(),
        };

        assert!(store.load_minio_credentials().unwrap().is_none());
        store.save_minio_credentials(&credentials).unwrap();
        assert_eq!(store.load_minio_credentials().unwrap(), Some(credentials));
        assert!(
            store
                .root()
                .join("providers/minio-main/credentials.json")
                .exists()
        );
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn initializes_default_provider_credentials() {
        let store = temp_store("default-provider-credentials");
        let credentials = store.load_or_init_default_provider_credentials().unwrap();

        assert_eq!(credentials.len(), 3);
        assert!(
            credentials
                .iter()
                .any(|(provider, _)| provider == "postgres.main")
        );
        assert!(
            credentials
                .iter()
                .any(|(provider, _)| provider == "redis.main")
        );
        assert!(
            credentials
                .iter()
                .any(|(provider, _)| provider == "minio.main")
        );
        assert!(
            store
                .root()
                .join("providers/postgres-main/credentials.json")
                .exists()
        );
        assert!(
            store
                .root()
                .join("providers/redis-main/credentials.json")
                .exists()
        );
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn initializes_provider_credentials_once() {
        let store = temp_store("provider-credentials-init");
        let first = store.load_or_init_minio_credentials().unwrap();
        assert_ne!(first.password, "gumgum-local-dev");
        assert_eq!(first.password.len(), 48);
        let mut changed = first.clone();
        changed.password = "changed".to_owned();
        store.save_minio_credentials(&changed).unwrap();
        assert_eq!(store.load_or_init_minio_credentials().unwrap(), changed);
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn stores_local_and_server_config_maps_separately() {
        let store = temp_store("config");
        let mut local = Map::new();
        local.insert("format".to_owned(), Value::String("json".to_owned()));
        store.save_config(&ConfigScope::Local, &local).unwrap();
        let mut server = Map::new();
        server.insert(
            "registry_port".to_owned(),
            Value::String("55000".to_owned()),
        );
        let server_scope = ConfigScope::Server("Starbase 2.local".to_owned());
        store.save_config(&server_scope, &server).unwrap();

        assert_eq!(
            store.load_config(&ConfigScope::Local).unwrap()["format"],
            "json"
        );
        assert_eq!(
            store.load_config(&server_scope).unwrap()["registry_port"],
            "55000"
        );
        assert!(
            store
                .root()
                .join("servers/starbase-2-local/config.json")
                .exists()
        );
        let _ = fs::remove_dir_all(store.root());
    }
}
