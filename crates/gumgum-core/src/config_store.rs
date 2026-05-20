use crate::{ErrorCode, GumgumError, Result, Subsystem, sanitize_name};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerRecord {
    pub name: String,
    pub host: String,
    pub root_domain: String,
    pub test_domain: String,
    pub health_url: String,
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

    fn ensure_parent(&self, path: &PathBuf) -> Result<()> {
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
    fn stores_local_and_server_config_maps_separately() {
        let store = temp_store("config");
        let mut local = Map::new();
        local.insert("format".to_owned(), Value::String("json".to_owned()));
        store.save_local_config(&local).unwrap();
        let mut server = Map::new();
        server.insert(
            "registry_port".to_owned(),
            Value::String("55000".to_owned()),
        );
        store
            .save_server_config("Starbase 2.local", &server)
            .unwrap();

        assert_eq!(store.load_local_config().unwrap()["format"], "json");
        assert_eq!(
            store.load_server_config("Starbase 2.local").unwrap()["registry_port"],
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
