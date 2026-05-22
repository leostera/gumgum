#![no_main]

use gumgum_api::{
    BindingDeleteRequest, BindingRequest, BucketObjectRequest, DeployRequest, DeploymentDeleteRequest,
    ObjectDeleteRequest, ObjectRequest, ProviderConfigureRequest, RollbackRequest,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let _ = serde_json::from_str::<DeployRequest>(input);
    let _ = serde_json::from_str::<DeploymentDeleteRequest>(input);
    let _ = serde_json::from_str::<BucketObjectRequest>(input);
    let _ = serde_json::from_str::<ObjectRequest>(input);
    let _ = serde_json::from_str::<ObjectDeleteRequest>(input);
    let _ = serde_json::from_str::<BindingRequest>(input);
    let _ = serde_json::from_str::<BindingDeleteRequest>(input);
    let _ = serde_json::from_str::<ProviderConfigureRequest>(input);
    let _ = serde_json::from_str::<RollbackRequest>(input);
});
