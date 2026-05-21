use crate::{ErrorCode, GumgumError, Subsystem};
use serde::{Deserialize, Serialize};

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
