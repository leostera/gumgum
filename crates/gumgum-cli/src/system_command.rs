use crate::{
    SchemaCommand, SchemaSubcommand, ServerCommand, ServerSubcommand, StatusArgs, config_command,
    print_value,
};
use gumgum_api::{PingReport, ServerListReport};
use gumgum_core::{
    ConfigStore, DaemonHealthClient, DaemonPingReport, DoctorCheck, DoctorReport,
    not_configured_status, validate_path,
};
use serde::Serialize;
use std::path::PathBuf;

pub(crate) async fn status(args: StatusArgs, json: bool) -> gumgum_core::Result<()> {
    if let Some(host) = args.host {
        let report = ping_host(&host).await?;
        print_value(json, &report)
    } else if let Some(server) = ConfigStore::from_home_env()?.load_default_server()? {
        let report = ping_host(&server.host).await?;
        print_value(json, &report)
    } else {
        print_value(json, &not_configured_status())
    }
    Ok(())
}

pub(crate) fn doctor(json: bool) {
    let report = DoctorReport {
        ok: true,
        checks: vec![
            DoctorCheck {
                name: "cli".to_owned(),
                ok: true,
                message: "gumgum CLI is installed".to_owned(),
            },
            DoctorCheck {
                name: "daemon".to_owned(),
                ok: true,
                message: "daemon check skipped until setup is implemented".to_owned(),
            },
        ],
    };
    print_value(json, &report)
}

pub(crate) fn version(json: bool) {
    print_value(json, &version_report());
}

pub(crate) async fn server(server: ServerCommand, json: bool) -> gumgum_core::Result<()> {
    match server.command {
        ServerSubcommand::List => {
            let report = ServerListReport {
                ok: true,
                servers: ConfigStore::from_home_env()?.load_servers()?,
            };
            print_value(json, &report)
        }
        ServerSubcommand::Ping(args) => {
            let report = ping_host(&args.host).await?;
            print_value(json, &report)
        }
        ServerSubcommand::Config(args) => {
            let report = config_command(Some(args.name), args.command)?;
            print_value(json, &report)
        }
    }
    Ok(())
}

pub(crate) fn schema(schema: SchemaCommand, json: bool) -> gumgum_core::Result<()> {
    match schema.command {
        SchemaSubcommand::Validate { path } => {
            let path = path.unwrap_or_else(|| PathBuf::from("gumgum.toml"));
            let report = validate_path(&path)?;
            print_value(json, &report)
        }
        SchemaSubcommand::Explain => {
            let explanation = SchemaExplanation {
                ok: true,
                schemas: vec!["workspace", "worker"],
                message: "v0 supports [workspace] and [worker] manifests".to_owned(),
            };
            print_value(json, &explanation)
        }
    }
    Ok(())
}

async fn ping_host(host: &str) -> gumgum_core::Result<PingReport> {
    Ok(ping_report_from_core(DaemonHealthClient::ping(host).await?))
}

fn ping_report_from_core(report: DaemonPingReport) -> PingReport {
    PingReport {
        ok: report.ok,
        host: report.host,
        health_url: report.health_url,
        service_active: report.service_active,
        health: report.health,
    }
}

#[derive(Debug, Serialize)]
struct VersionReport {
    ok: bool,
    version: &'static str,
    git_sha: &'static str,
    target: &'static str,
}

fn version_report() -> VersionReport {
    VersionReport {
        ok: true,
        version: option_env!("GUMGUM_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
        git_sha: option_env!("GUMGUM_BUILD_SHA").unwrap_or("unknown"),
        target: option_env!("GUMGUM_BUILD_TARGET").unwrap_or("unknown"),
    }
}

#[derive(Debug, Serialize)]
struct SchemaExplanation {
    ok: bool,
    schemas: Vec<&'static str>,
    message: String,
}
