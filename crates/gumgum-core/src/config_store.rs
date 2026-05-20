use crate::{ErrorCode, GumgumError, Result, Subsystem};
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

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
