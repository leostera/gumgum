use crate::{
    BindObjectArgs, CreateObjectArgs, ObjectArgs, ObjectCommand, print_value, resolve_server,
};
use gumgum_api::{BindingRequest, ObjectReport, ObjectRequest};
use gumgum_core::{Capability, load_worker_path};
use std::path::PathBuf;

use crate::server_client::ServerClient;

pub(crate) async fn object_command(
    kind: &str,
    args: ObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let capability = capability_from_cli_kind(kind);
    match args.command {
        ObjectCommand::Create(args) => create_object(capability, args, json).await,
        ObjectCommand::Bind(args) => bind_object(capability, args, json).await,
    }
}

fn capability_from_cli_kind(kind: &str) -> Capability {
    match kind {
        "db" => Capability::Db,
        "kv" => Capability::Kv,
        "bucket" | "blob" => Capability::Blob,
        "queue" => Capability::Queue,
        "secret" | "secrets" => Capability::Secret,
        _ => Capability::Manual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_commands_map_to_provider_capabilities() {
        assert_eq!(capability_from_cli_kind("db"), Capability::Db);
        assert_eq!(capability_from_cli_kind("kv"), Capability::Kv);
        assert_eq!(capability_from_cli_kind("bucket"), Capability::Blob);
        assert_eq!(capability_from_cli_kind("queue"), Capability::Queue);
        assert_eq!(capability_from_cli_kind("secret"), Capability::Secret);
    }
}

async fn create_object(
    capability: Capability,
    args: CreateObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let root_domain = args
        .root_domain
        .unwrap_or_else(|| server.root_domain.clone());
    let request = ObjectRequest {
        capability,
        name: args.name,
        namespace: args.namespace,
        root_domain,
    };
    let report: ObjectReport = ServerClient::new(server.host)
        .create_object(&request)
        .await?;
    if json {
        print_value(true, &report);
    } else {
        print_object_report(&report);
    }
    Ok(())
}

fn print_object_report(report: &ObjectReport) {
    crate::presentation::Presenter::new().object_report(report);
}

async fn bind_object(
    capability: Capability,
    args: BindObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let worker = match args.to {
        Some(worker) => worker,
        None => load_worker_path(&PathBuf::from("gumgum.toml"))?.worker.name,
    };
    let request = BindingRequest {
        capability,
        object_name: args.name,
        worker,
        binding: args.binding,
        access: args.access,
    };
    let report = ServerClient::new(server.host).bind_object(&request).await?;
    if json {
        print_value(true, &report);
    } else {
        println!(
            "bound {} to {} as {}",
            report.object, report.worker, report.binding
        );
    }
    Ok(())
}
