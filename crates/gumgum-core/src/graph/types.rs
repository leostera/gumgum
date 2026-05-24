use crate::{ErrorCode, ErrorKind, GumgumError, Subsystem, sanitize_name};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Port(u16);

impl Port {
    pub fn new(value: u16) -> crate::Result<Self> {
        if value == 0 {
            Err(invalid_graph_value())
        } else {
            Ok(Self(value))
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl From<Port> for u16 {
    fn from(value: Port) -> Self {
        value.0
    }
}

impl TryFrom<u16> for Port {
    type Error = GumgumError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct WorkerId(String);

impl WorkerId {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = sanitize_name(value.as_ref());
        if value.is_empty() {
            Err(invalid_graph_value())
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for WorkerId {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for WorkerId {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContainerName(String);

impl ContainerName {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = sanitize_name(value.as_ref());
        if value.is_empty() {
            Err(invalid_graph_value())
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContainerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ContainerName {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ContainerName {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ImageName(String);

impl ImageName {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ImageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ImageName {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ImageName {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RouteHost(String);

impl RouteHost {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value
            .as_ref()
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) || !value.contains('.') {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RouteHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RouteHost {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for RouteHost {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct HealthPath(String);

impl HealthPath {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if !value.starts_with('/') {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HealthPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for HealthPath {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for HealthPath {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProviderName(String);

impl ProviderName {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ProviderName {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ProviderName {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ObjectName(String);

impl ObjectName {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ObjectName {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ObjectName {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct BindingName(String);

impl BindingName {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BindingName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for BindingName {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for BindingName {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ObjectRef(String);

impl ObjectRef {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) || !value.contains('/') {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ObjectRef {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ObjectRef {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct GraphNodeId(String);

impl GraphNodeId {
    pub fn new(value: impl AsRef<str>) -> crate::Result<Self> {
        let value = value.as_ref().trim().to_owned();
        if value.is_empty() {
            return Err(invalid_graph_value());
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GraphNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for GraphNodeId {
    type Error = GumgumError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for GraphNodeId {
    type Error = GumgumError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

fn invalid_graph_value() -> GumgumError {
    GumgumError::structured_kind(
        Subsystem::Config,
        ErrorCode::InvalidArgs,
        ErrorKind::GraphValueInvalid,
    )
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_id_is_sanitized_and_non_empty() {
        assert_eq!(WorkerId::new("API Worker").unwrap().as_str(), "api-worker");
        assert!(WorkerId::new("---").is_err());
    }

    #[test]
    fn container_name_is_sanitized_and_non_empty() {
        assert_eq!(
            ContainerName::new("gumgum_API.1").unwrap().as_str(),
            "gumgum-api-1"
        );
        assert!(ContainerName::new("---").is_err());
    }

    #[test]
    fn image_name_preserves_registry_reference_but_rejects_whitespace() {
        let image = ImageName::new(" ghcr.io/acme/api:v1 ").unwrap();
        assert_eq!(image.as_str(), "ghcr.io/acme/api:v1");
        assert!(ImageName::new("bad image:v1").is_err());
        assert!(ImageName::new(" ").is_err());
    }

    #[test]
    fn route_host_normalizes_and_requires_dotted_hostname() {
        assert_eq!(
            RouteHost::new("API.Example.Test.").unwrap().as_str(),
            "api.example.test"
        );
        assert!(RouteHost::new("localhost").is_err());
        assert!(RouteHost::new("bad host.test").is_err());
    }

    #[test]
    fn health_path_requires_absolute_path_without_whitespace() {
        assert_eq!(HealthPath::new(" /healthz ").unwrap().as_str(), "/healthz");
        assert!(HealthPath::new("healthz").is_err());
        assert!(HealthPath::new("/health z").is_err());
        assert!(HealthPath::new(" ").is_err());
    }

    #[test]
    fn provider_and_object_names_reject_empty_or_whitespace() {
        assert_eq!(
            ProviderName::new(" secrets.platform ").unwrap().as_str(),
            "secrets.platform"
        );
        assert_eq!(
            ObjectName::new(" peekaboo-assets ").unwrap().as_str(),
            "peekaboo-assets"
        );
        assert!(ProviderName::new("bad provider").is_err());
        assert!(ObjectName::new(" ").is_err());
    }

    #[test]
    fn binding_names_and_object_refs_reject_invalid_values() {
        assert_eq!(
            BindingName::new(" DATABASE_URL ").unwrap().as_str(),
            "DATABASE_URL"
        );
        assert_eq!(ObjectRef::new(" db/main ").unwrap().as_str(), "db/main");
        assert!(BindingName::new("DATABASE URL").is_err());
        assert!(ObjectRef::new("main").is_err());
        assert!(ObjectRef::new("db/main secret").is_err());
    }

    #[test]
    fn graph_node_id_rejects_empty_or_whitespace() {
        assert_eq!(
            GraphNodeId::new(" deployment/api ").unwrap().as_str(),
            "deployment/api"
        );
        assert!(GraphNodeId::new("deployment api").is_err());
        assert!(GraphNodeId::new(" ").is_err());
    }

    #[test]
    fn port_rejects_zero() {
        assert!(Port::new(0).is_err());
        assert_eq!(Port::new(3000).unwrap().get(), 3000);
    }
}
