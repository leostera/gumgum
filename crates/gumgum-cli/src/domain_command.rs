use crate::{
    DomainAddArgs, DomainArgs, DomainCommand,
    presentation::{action_text, action_texts},
    print_value, resolve_server,
};
use gumgum_api::{DomainAddRequest, DomainReport};
use gumgum_core::{
    CloudflarePermissionGrant, CloudflarePermissionScope, CloudflarePermissionTarget,
    DomainProvider, IngressMode, cloudflare,
};
use serde::Serialize;
use std::io::{self, Write};

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
    let mut actions = vec![gumgum_core::CoreAction::CliMessage {
        message: format!(
            "record domain {} on server {} with {} provider",
            args.name,
            server.name,
            provider.as_str()
        ),
    }];
    if provider == DomainProvider::Cloudflare {
        actions.push(gumgum_core::CoreAction::CliMessage {
            message: format!("authorize Cloudflare for {}", args.name),
        });
    }
    if ingress == IngressMode::Cloudflare {
        actions.push(gumgum_core::CoreAction::CliMessage {
            message: "configure Cloudflare tunnel ingress".to_owned(),
        });
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
    let client = ServerClient::new(server.host);
    let mut report = client
        .add_domain(&DomainAddRequest {
            name: args.name.clone(),
            provider,
            ingress,
            cloudflare_grant: None,
        })
        .await?;
    if !report.ok
        && provider == DomainProvider::Cloudflare
        && report.actions.iter().any(|action| {
            let text = action_text(action);
            text == "Cloudflare grant is required"
                || text.starts_with("Cloudflare zone verification failed:")
        })
    {
        if !json {
            for action in &report.actions {
                println!("→ {}", action_text(action));
            }
            if report.actions.iter().any(|action| {
                action_text(action).starts_with("Cloudflare zone verification failed:")
            }) {
                println!(
                    "The saved Cloudflare token cannot see {}. Create or update a token that includes this domain, then paste it below.",
                    args.name
                );
            }
        }
        let grant = authorize_cloudflare_zone(&args.name)?;
        report = client
            .add_domain(&DomainAddRequest {
                name: args.name,
                provider,
                ingress,
                cloudflare_grant: Some(grant),
            })
            .await?;
    }
    if !report.ok {
        return Err(gumgum_core::GumgumError::structured(
            gumgum_core::Subsystem::Config,
            gumgum_core::ErrorCode::Io,
            report.message,
        )
        .likely_cause(action_texts(&report.actions).join("; "))
        .build());
    }
    if !json {
        for action in &report.actions {
            println!("→ {}", action_text(action));
        }
        println!("{}", report.message);
    }
    Ok(report)
}

fn cloudflare_scope_text(scope: CloudflarePermissionScope) -> &'static str {
    match scope {
        CloudflarePermissionScope::Zone => "Zone",
        CloudflarePermissionScope::Account => "Account",
    }
}

fn cloudflare_permission_text(permission: CloudflarePermissionGrant) -> &'static str {
    match permission {
        CloudflarePermissionGrant::ZoneRead => "Zone Read",
        CloudflarePermissionGrant::DnsWrite => "DNS Write",
        CloudflarePermissionGrant::CloudflaredWrite => {
            "Cloudflare One Connector: cloudflared Write"
        }
    }
}

fn cloudflare_target_text(target: CloudflarePermissionTarget) -> &'static str {
    match target {
        CloudflarePermissionTarget::ManagedZones => "All zones GumGum should manage",
        CloudflarePermissionTarget::TunnelIngressAccount => {
            "Account used for Cloudflare Tunnel ingress"
        }
    }
}

pub(crate) fn authorize_cloudflare_zone(
    zone_name: &str,
) -> gumgum_core::Result<gumgum_core::CloudflareGrant> {
    let prompt = cloudflare::token_prompt(zone_name);
    eprintln!("Cloudflare API token required for {}.", prompt.zone_name);
    eprintln!("Create a Cloudflare API token with these permission policies:");
    eprintln!("  Scope    Permission                                      Applies to");
    eprintln!(
        "  -------  ----------------------------------------------  ---------------------------------------------------------------"
    );
    for permission in &prompt.permissions {
        eprintln!(
            "  {:<7}  {:<46}  {}",
            cloudflare_scope_text(permission.scope),
            cloudflare_permission_text(permission.permission),
            cloudflare_target_text(permission.applies_to)
        );
    }
    eprintln!("Use Cloudflare's token builder as:");
    eprintln!("  All zones in your account: Zone Read, DNS Write");
    eprintln!("  Entire account: Cloudflare One Connector: cloudflared Write");
    eprintln!("Make sure the zone list includes: {}", prompt.zone_name);
    eprintln!(
        "When adding more Cloudflare-managed domains later, update/recreate the token to include those domains too."
    );
    eprintln!("Cloudflare token page: {}", prompt.token_url);
    eprint!("Paste Cloudflare API token: ");
    io::stderr().flush().map_err(|source| {
        gumgum_core::GumgumError::structured(
            gumgum_core::Subsystem::Config,
            gumgum_core::ErrorCode::Io,
            "could not flush prompt",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let mut token = String::new();
    io::stdin().read_line(&mut token).map_err(|source| {
        gumgum_core::GumgumError::structured(
            gumgum_core::Subsystem::Config,
            gumgum_core::ErrorCode::Io,
            "could not read Cloudflare token",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    cloudflare::grant_from_api_token(zone_name, token)
}
