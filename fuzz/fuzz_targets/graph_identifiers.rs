#![no_main]

use gumgum_core::{
    BindingName, ContainerName, GraphNodeId, HealthPath, ImageName, ObjectName, ObjectRef, Port,
    ProviderName, RouteHost, WorkerId,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = BindingName::new(input);
        let _ = ContainerName::new(input);
        let _ = GraphNodeId::new(input);
        let _ = HealthPath::new(input);
        let _ = ImageName::new(input);
        let _ = ObjectName::new(input);
        let _ = ObjectRef::new(input);
        let _ = ProviderName::new(input);
        let _ = RouteHost::new(input);
        let _ = WorkerId::new(input);
    }

    if data.len() >= 2 {
        let port = u16::from_le_bytes([data[0], data[1]]);
        let _ = Port::new(port);
    }
});
