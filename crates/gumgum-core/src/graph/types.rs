use crate::{ErrorCode, GumgumError, Subsystem, sanitize_name};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Port(u16);

impl Port {
    pub fn new(value: u16) -> crate::Result<Self> {
        if value == 0 {
            Err(GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "port must be between 1 and 65535",
            )
            .build())
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
            Err(invalid_graph_value("worker id must not be empty"))
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
            Err(invalid_graph_value("container name must not be empty"))
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
            return Err(invalid_graph_value("image name must not be empty"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid_graph_value(
                "image name must not contain whitespace",
            ));
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
            return Err(invalid_graph_value("route host must not be empty"));
        }
        if value.chars().any(char::is_whitespace) || !value.contains('.') {
            return Err(invalid_graph_value(
                "route host must be a dotted hostname without whitespace",
            ));
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

fn invalid_graph_value(message: &'static str) -> GumgumError {
    GumgumError::structured(Subsystem::Config, ErrorCode::InvalidArgs, message).build()
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
    fn port_rejects_zero() {
        assert!(Port::new(0).is_err());
        assert_eq!(Port::new(3000).unwrap().get(), 3000);
    }
}
