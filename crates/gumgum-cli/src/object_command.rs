#![allow(clippy::items_after_test_module)]

use crate::{
    BindObjectArgs, BucketArgs, BucketCommand, BucketCopyArgs, BucketPathArgs, CreateObjectArgs,
    DeleteObjectArgs, ListObjectArgs, ObjectArgs, ObjectCommand, UnbindObjectArgs,
    bucket_paths::{is_local_bucket_path, split_remote_bucket_path},
    print_value, resolve_server,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use gumgum_api::GraphReport;
use gumgum_api::{
    BindingDeleteRequest, BindingRequest, BucketObjectRequest, ObjectDeleteRequest, ObjectReport,
    ObjectRequest,
};
use gumgum_core::{Capability, ErrorCode, GumgumError, Subsystem, load_worker_path};
use serde::Serialize;
use std::path::PathBuf;

use crate::{presentation::action_text, server_client::ServerClient};

pub(crate) async fn object_command(
    kind: &str,
    args: ObjectArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let capability = capability_from_cli_kind(kind);
    match args.command {
        ObjectCommand::List(args) => list_objects(capability, args, json).await,
        ObjectCommand::Create(args) => create_object(capability, args, json, dry_run).await,
        ObjectCommand::Delete(args) => delete_object(capability, args, json, dry_run).await,
        ObjectCommand::Bind(args) => bind_object(capability, args, json, dry_run).await,
        ObjectCommand::Unbind(args) => unbind_object(capability, args, json, dry_run).await,
    }
}

pub(crate) async fn bucket_command(
    args: BucketArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let capability = Capability::Blob;
    match args.command {
        BucketCommand::List(args) => list_objects(capability, args, json).await,
        BucketCommand::Create(args) => create_object(capability, args, json, dry_run).await,
        BucketCommand::Delete(args) => delete_object(capability, args, json, dry_run).await,
        BucketCommand::Bind(args) => bind_object(capability, args, json, dry_run).await,
        BucketCommand::Unbind(args) => unbind_object(capability, args, json, dry_run).await,
        BucketCommand::Ls(args) => bucket_path_command("ls", args, json).await,
        BucketCommand::Get(args) => bucket_path_command("get", args, json).await,
        BucketCommand::Rm(args) => bucket_path_command("rm", args, json).await,
        BucketCommand::Cp(args) => bucket_copy_command("cp", args, json).await,
        BucketCommand::Sync(args) => bucket_copy_command("sync", args, json).await,
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

fn graph_object_kind(capability: Capability) -> &'static str {
    match capability {
        Capability::Db => "db",
        Capability::Kv => "kv",
        Capability::Blob => "bucket",
        Capability::Queue => "queue",
        Capability::Secret => "secret",
        _ => "manual",
    }
}

#[derive(Clone, Debug, Serialize)]
struct ObjectListReport {
    ok: bool,
    kind: String,
    objects: Vec<ObjectListEntry>,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct ObjectListEntry {
    kind: String,
    name: String,
    label: String,
    bound_to: Vec<String>,
}

async fn list_objects(
    capability: Capability,
    args: ListObjectArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let graph = ServerClient::new(server.host).graph().await?;
    let kind = graph_object_kind(capability);
    let report = object_list_report(kind, &graph);
    if json {
        print_value(true, &report);
    } else {
        print_object_list_report(&report);
    }
    Ok(())
}

fn object_list_report(kind: &str, graph: &GraphReport) -> ObjectListReport {
    let mut objects = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "global_object" || node.kind == "object")
        .filter_map(|node| {
            let (node_kind, name) = node.id.split_once('/')?;
            (node_kind == kind).then(|| ObjectListEntry {
                kind: node_kind.to_owned(),
                name: name.to_owned(),
                label: node.label.clone(),
                bound_to: object_bindings(&node.id, graph),
            })
        })
        .collect::<Vec<_>>();
    objects.sort_by(|left, right| left.name.cmp(&right.name));
    ObjectListReport {
        ok: true,
        kind: kind.to_owned(),
        message: format!("{} {} object(s)", objects.len(), kind),
        objects,
    }
}

fn object_bindings(object_id: &str, graph: &GraphReport) -> Vec<String> {
    let mut bindings = Vec::new();
    for edge in graph.edges.iter().filter(|edge| {
        edge.kind == "projects_as"
            && (edge.to == object_id || edge.to == format!("object/{object_id}"))
    }) {
        if let Some(binding) = edge.from.strip_prefix("binding/") {
            bindings.push(binding.replace('/', "."));
        }
    }
    bindings.sort();
    bindings.dedup();
    bindings
}

fn print_object_list_report(report: &ObjectListReport) {
    if report.objects.is_empty() {
        println!("No {} objects found.", report.kind);
        return;
    }
    println!("{:<24} {:<32} BOUND", "NAME", "LABEL");
    for object in &report.objects {
        let bound = if object.bound_to.is_empty() {
            "-".to_owned()
        } else {
            object.bound_to.join(", ")
        };
        println!("{:<24} {:<32} {}", object.name, object.label, bound);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{GraphEdge, GraphNode};

    #[test]
    fn object_commands_map_to_provider_capabilities() {
        assert_eq!(capability_from_cli_kind("db"), Capability::Db);
        assert_eq!(capability_from_cli_kind("kv"), Capability::Kv);
        assert_eq!(capability_from_cli_kind("bucket"), Capability::Blob);
        assert_eq!(capability_from_cli_kind("queue"), Capability::Queue);
        assert_eq!(capability_from_cli_kind("secret"), Capability::Secret);
    }

    #[test]
    fn object_list_report_filters_graph_objects_and_bindings() {
        let graph = GraphReport {
            ok: true,
            format: "json".to_owned(),
            graph: String::new(),
            nodes: vec![
                GraphNode::new("db/visits", "global_object", "db: visits.db.test"),
                GraphNode::new("kv/users", "global_object", "kv: users.kv.test"),
            ],
            edges: vec![GraphEdge {
                from: "binding/api/DATABASE_URL".to_owned(),
                to: "db/visits".to_owned(),
                kind: "projects_as".to_owned(),
            }],
        };

        let report = object_list_report("db", &graph);

        assert_eq!(report.objects.len(), 1);
        assert_eq!(report.objects[0].name, "visits");
        assert_eq!(report.objects[0].bound_to, vec!["api.DATABASE_URL"]);
    }
}

async fn bucket_path_command(
    action: &str,
    args: BucketPathArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host)
        .bucket_object(
            action,
            &BucketObjectRequest {
                bucket: Some(args.bucket),
                path: args.path,
                source: None,
                destination: None,
                content_base64: None,
            },
        )
        .await?;
    if json {
        print_value(true, &report);
    } else {
        print_bucket_object_report(&report);
    }
    Ok(())
}

async fn bucket_copy_command(
    action: &str,
    args: BucketCopyArgs,
    json: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let client = ServerClient::new(server.host);
    let source_is_local = is_local_bucket_path(&args.source);
    let destination_is_local = is_local_bucket_path(&args.destination);
    if action == "cp" && source_is_local && !destination_is_local {
        let content = std::fs::read(&args.source).map_err(|source| {
            GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::Io,
                "could not read local bucket source",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        let report = client
            .bucket_object(
                action,
                &BucketObjectRequest {
                    bucket: None,
                    path: None,
                    source: Some(args.source),
                    destination: Some(args.destination),
                    content_base64: Some(BASE64.encode(content)),
                },
            )
            .await?;
        return print_bucket_command_report(json, &report);
    }
    if action == "cp" && !source_is_local && destination_is_local {
        let (bucket, path) = split_remote_bucket_path(&args.source)?;
        let report = client
            .bucket_object(
                "get",
                &BucketObjectRequest {
                    bucket: Some(bucket),
                    path: Some(path),
                    source: None,
                    destination: None,
                    content_base64: None,
                },
            )
            .await?;
        let content = report.content_base64.as_ref().ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "bucket object response had no content",
            )
            .build()
        })?;
        let bytes = BASE64.decode(content).map_err(|source| {
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::InvalidArgs,
                "bucket object content is not valid base64",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        std::fs::write(&args.destination, bytes).map_err(|source| {
            GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::Io,
                "could not write local bucket destination",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
        if json {
            print_value(true, &report);
        } else {
            println!("copied {} to {}", args.source, args.destination);
        }
        return Ok(());
    }
    if source_is_local || destination_is_local {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            "bucket sync only supports remote bucket paths; use cp for local transfers",
        )
        .next_command("gumgum bucket cp <local> <bucket/key>")
        .next_command("gumgum bucket cp <bucket/key> <local>")
        .build());
    }
    let report = client
        .bucket_object(
            action,
            &BucketObjectRequest {
                bucket: None,
                path: None,
                source: Some(args.source),
                destination: Some(args.destination),
                content_base64: None,
            },
        )
        .await?;
    print_bucket_command_report(json, &report)
}

fn print_bucket_command_report(
    json: bool,
    report: &gumgum_api::BucketObjectReport,
) -> gumgum_core::Result<()> {
    if json {
        print_value(true, report);
    } else {
        print_bucket_object_report(report);
    }
    Ok(())
}

fn print_bucket_object_report(report: &gumgum_api::BucketObjectReport) {
    if let Some(content) = &report.content {
        print!("{content}");
        return;
    }
    if !report.objects.is_empty() {
        for object in &report.objects {
            println!("{object}");
        }
        return;
    }
    for action in &report.actions {
        println!("{}", action_text(action));
    }
    println!("{}", report.message);
}

async fn create_object(
    capability: Capability,
    args: CreateObjectArgs,
    json: bool,
    dry_run: bool,
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
        password: args.password,
        preview: dry_run,
    };
    let client = ServerClient::new(server.host.clone());
    if dry_run {
        require_preview_capability(&client, &server.name, "gumgum:objects:create_preview").await?;
    }
    let report: ObjectReport = client.create_object(&request).await?;
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

async fn require_preview_capability(
    client: &ServerClient,
    server_name: &str,
    capability: &str,
) -> gumgum_core::Result<()> {
    let report = client.version().await?;
    if report
        .capabilities
        .iter()
        .any(|present| present == capability)
    {
        return Ok(());
    }
    Err(GumgumError::structured(
        Subsystem::Api,
        ErrorCode::InvalidArgs,
        format!("gumgumd on {server_name} does not support safe {capability}"),
    )
    .likely_cause("older gumgumd ignores create-preview fields and may mutate during --dry-run")
    .next_command(format!("gumgum server {server_name} upgrade"))
    .next_command(format!(
        "gumgum server {server_name} capabilities --require {capability}"
    ))
    .build())
}

async fn delete_object(
    capability: Capability,
    args: DeleteObjectArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let root_domain = args
        .root_domain
        .unwrap_or_else(|| server.root_domain.clone());
    let request = ObjectDeleteRequest {
        capability,
        name: args.name,
        namespace: args.namespace,
        root_domain,
        preview: args.preview || dry_run,
    };
    let report = ServerClient::new(server.host)
        .delete_object(&request)
        .await?;
    if json {
        print_value(true, &report);
    } else {
        println!("{} {}", report.kind, report.message);
    }
    Ok(())
}

async fn unbind_object(
    capability: Capability,
    args: UnbindObjectArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let server = resolve_server(args.host)?;
    let worker = match args.to {
        Some(worker) => worker,
        None => load_worker_path(&PathBuf::from("gumgum.toml"))?.worker.name,
    };
    let request = BindingDeleteRequest {
        capability,
        object_name: args.name,
        worker,
        binding: args.binding,
        preview: args.preview || dry_run,
    };
    let report = ServerClient::new(server.host)
        .delete_binding(&request)
        .await?;
    if json {
        print_value(true, &report);
    } else {
        if args.preview || dry_run {
            println!(
                "would unbind {} from {} as {}",
                report.object, report.worker, report.binding
            );
        } else {
            println!(
                "unbound {} from {} as {}",
                report.object, report.worker, report.binding
            );
        }
    }
    Ok(())
}

async fn bind_object(
    capability: Capability,
    args: BindObjectArgs,
    json: bool,
    dry_run: bool,
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
        preview: dry_run,
    };
    let client = ServerClient::new(server.host.clone());
    if dry_run {
        require_preview_capability(&client, &server.name, "gumgum:bindings:create_preview").await?;
    }
    let report = client.bind_object(&request).await?;
    if json {
        print_value(true, &report);
    } else if dry_run {
        println!(
            "would bind {} to {} as {}",
            report.object, report.worker, report.binding
        );
    } else {
        println!(
            "bound {} to {} as {}",
            report.object, report.worker, report.binding
        );
    }
    Ok(())
}
