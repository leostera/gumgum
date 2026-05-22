use crate::{DomainAddArgs, DomainArgs, DomainCommand, print_value, resolve_server};
use gumgum_api::{DomainAddRequest, DomainReport};
use gumgum_core::{DomainProvider, IngressMode, cloudflare};
use serde::Serialize;

use crate::server_client::ServerClient;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum DomainOutput {
    Add(DomainReport),
}

pub(crate) async fn domain_command(
    args: DomainArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<()> {
    let output = match args.command {
        DomainCommand::Add(args) => DomainOutput::Add(add_domain(args, json, dry_run).await?),
    };
    print_value(json, &output);
    Ok(())
}

pub(crate) async fn add_domain(
    args: DomainAddArgs,
    json: bool,
    dry_run: bool,
) -> gumgum_core::Result<DomainReport> {
    let server = resolve_server(args.server)?;
    let provider: DomainProvider = args.provider.into();
    let ingress: IngressMode = args.ingress.into();
    let mut actions = vec![format!(
        "record domain {} on server {} with {} provider",
        args.name,
        server.name,
        provider.as_str()
    )];
    if provider == DomainProvider::Cloudflare {
        actions.push(format!("authorize Cloudflare for {}", args.name));
    }
    if ingress == IngressMode::Cloudflare {
        actions.push("configure Cloudflare tunnel ingress".to_owned());
    }
    if dry_run {
        return Ok(DomainReport {
            ok: true,
            name: args.name,
            provider,
            ingress,
            actions,
            message: "domain add plan".to_owned(),
        });
    }
    let grant = if provider == DomainProvider::Cloudflare {
        Some(cloudflare::authorize_zone(&args.name).await?)
    } else {
        None
    };
    let report = ServerClient::new(server.host)
        .add_domain(&DomainAddRequest {
            name: args.name,
            provider,
            ingress,
            cloudflare_grant: grant,
        })
        .await?;
    if !report.ok {
        return Err(gumgum_core::GumgumError::structured(
            gumgum_core::Subsystem::Config,
            gumgum_core::ErrorCode::Io,
            report.message,
        )
        .likely_cause(report.actions.join("; "))
        .build());
    }
    if !json {
        for action in &report.actions {
            println!("→ {action}");
        }
        println!("{}", report.message);
    }
    Ok(report)
}
