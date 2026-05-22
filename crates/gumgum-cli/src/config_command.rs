use crate::ConfigSubcommand;
use gumgum_core::{ConfigScope, ConfigStore, ErrorCode, GumgumError, Subsystem};
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Serialize)]
pub(crate) struct ConfigReport {
    ok: bool,
    scope: String,
    values: Map<String, Value>,
    entries: Vec<ConfigEntry>,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConfigEntry {
    key: String,
    value: Value,
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
        ConfigSubcommand::List => Ok(report(scope, values, "config values")),
        ConfigSubcommand::Get { key } => {
            let schema = require_known_config_key(&key)?;
            let mut selected = Map::new();
            if let Some(value) = get_dotted(&values, schema.key) {
                set_dotted_value(&mut selected, schema.key, value.clone());
            }
            Ok(report(scope, selected, "config value"))
        }
        ConfigSubcommand::Set { key, value } => {
            let schema = require_known_config_key(&key)?;
            let value = schema.parse(&value)?;
            set_dotted_value(&mut values, schema.key, value.clone());
            store.save_config(&config_scope, &values)?;
            let mut selected = Map::new();
            set_dotted_value(&mut selected, schema.key, value);
            Ok(report(scope, selected, "config value saved"))
        }
    }
}

pub(crate) fn print_config_report(report: &ConfigReport) {
    println!("Scope: {}", report.scope);
    if report.entries.is_empty() {
        println!("No config values set.");
        println!("Known keys:");
        for schema in known_config_keys() {
            println!("  - {} ({})", schema.key, schema.kind);
        }
        return;
    }
    println!("{:<24} VALUE", "KEY");
    for entry in &report.entries {
        println!("{:<24} {}", entry.key, display_value(&entry.value));
    }
}

fn report(scope: String, values: Map<String, Value>, message: &str) -> ConfigReport {
    let entries = flatten_config(&values);
    ConfigReport {
        ok: true,
        scope,
        values,
        entries,
        message: message.to_owned(),
    }
}

#[derive(Clone, Copy, Debug)]
struct ConfigSchema {
    key: &'static str,
    kind: &'static str,
}

impl ConfigSchema {
    fn parse(self, value: &str) -> gumgum_core::Result<Value> {
        match self.kind {
            "bool" => value.parse::<bool>().map(Value::Bool).map_err(|_| {
                GumgumError::structured(
                    Subsystem::Config,
                    ErrorCode::InvalidArgs,
                    format!("{} expects true or false", self.key),
                )
                .build()
            }),
            "number" => value
                .parse::<u16>()
                .map(|number| Value::from(number as u64))
                .map_err(|_| {
                    GumgumError::structured(
                        Subsystem::Config,
                        ErrorCode::InvalidArgs,
                        format!("{} expects a number", self.key),
                    )
                    .build()
                }),
            _ => Ok(Value::String(value.to_owned())),
        }
    }
}

fn require_known_config_key(key: &str) -> gumgum_core::Result<ConfigSchema> {
    known_config_keys()
        .into_iter()
        .find(|schema| schema.key == key)
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                format!("unknown GumGum config key {key}"),
            )
            .likely_cause("gumgum config is schema-backed, not an arbitrary key/value store")
            .next_command("gumgum config list")
            .build()
        })
}

fn known_config_keys() -> Vec<ConfigSchema> {
    vec![
        ConfigSchema {
            key: "ui.color",
            kind: "bool",
        },
        ConfigSchema {
            key: "format",
            kind: "string",
        },
        ConfigSchema {
            key: "registry_port",
            kind: "number",
        },
    ]
}

fn flatten_config(values: &Map<String, Value>) -> Vec<ConfigEntry> {
    let mut entries = Vec::new();
    for (key, value) in values {
        flatten_value(key, value, &mut entries);
    }
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries
}

fn flatten_value(prefix: &str, value: &Value, entries: &mut Vec<ConfigEntry>) {
    if let Value::Object(map) = value {
        for (key, value) in map {
            flatten_value(&format!("{prefix}.{key}"), value, entries);
        }
    } else {
        entries.push(ConfigEntry {
            key: prefix.to_owned(),
            value: value.clone(),
        });
    }
}

fn get_dotted<'a>(values: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    let mut parts = key.split('.');
    let first = parts.next()?;
    let mut value = values.get(first)?;
    for part in parts {
        value = value.as_object()?.get(part)?;
    }
    Some(value)
}

fn set_dotted_value(values: &mut Map<String, Value>, key: &str, value: Value) {
    let parts = key.split('.').collect::<Vec<_>>();
    set_dotted_parts(values, &parts, value);
}

fn set_dotted_parts(values: &mut Map<String, Value>, parts: &[&str], value: Value) {
    if parts.len() == 1 {
        values.insert(parts[0].to_owned(), value);
        return;
    }
    let entry = values
        .entry(parts[0].to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    set_dotted_parts(
        entry.as_object_mut().expect("object just inserted"),
        &parts[1..],
        value,
    );
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_nested_config_as_dotted_paths() {
        let mut values = Map::new();
        set_dotted_value(&mut values, "ui.color", Value::Bool(true));
        set_dotted_value(&mut values, "registry_port", Value::from(5000));

        let entries = flatten_config(&values);

        assert_eq!(entries[0].key, "registry_port");
        assert_eq!(entries[1].key, "ui.color");
        assert_eq!(display_value(&entries[1].value), "true");
    }

    #[test]
    fn rejects_unknown_config_keys() {
        let report = require_known_config_key("anything.goes")
            .unwrap_err()
            .to_report();
        assert!(report.message.contains("unknown GumGum config key"));
    }

    #[test]
    fn parses_known_config_values_by_schema() {
        assert_eq!(
            require_known_config_key("ui.color")
                .unwrap()
                .parse("true")
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            require_known_config_key("registry_port")
                .unwrap()
                .parse("5000")
                .unwrap(),
            Value::from(5000_u64)
        );
    }
}
