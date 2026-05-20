use crate::ConfigSubcommand;
use gumgum_core::{ConfigScope, ConfigStore};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ConfigReport {
    ok: bool,
    scope: String,
    values: serde_json::Map<String, serde_json::Value>,
    message: String,
}

pub(crate) fn config_command(
    server_name: Option<String>,
    command: ConfigSubcommand,
) -> gumgum_core::Result<ConfigReport> {
    let store = ConfigStore::from_home_env()?;
    let config_scope = server_name
        .map(ConfigScope::Server)
        .unwrap_or(ConfigScope::Local);
    let scope = config_scope.label();
    let mut values = store.load_config(&config_scope)?;
    match command {
        ConfigSubcommand::List => Ok(ConfigReport {
            ok: true,
            scope,
            values,
            message: "config values".to_owned(),
        }),
        ConfigSubcommand::Get { key } => {
            let mut selected = serde_json::Map::new();
            if let Some(value) = values.get(&key) {
                selected.insert(key, value.clone());
            }
            Ok(ConfigReport {
                ok: true,
                scope,
                values: selected,
                message: "config value".to_owned(),
            })
        }
        ConfigSubcommand::Set { key, value } => {
            values.insert(key.clone(), serde_json::Value::String(value));
            store.save_config(&config_scope, &values)?;
            let mut selected = serde_json::Map::new();
            selected.insert(key.clone(), values.get(&key).cloned().unwrap());
            Ok(ConfigReport {
                ok: true,
                scope,
                values: selected,
                message: "config value saved".to_owned(),
            })
        }
    }
}
