use gumgum_core::GumgumError;
use serde::Serialize;

pub(crate) fn progress(quiet: bool, message: impl AsRef<str>) {
    if !quiet {
        eprintln!("→ {}", message.as_ref());
    }
}

pub(crate) fn print_value<T: Serialize>(_json: bool, value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialize json")
    );
}

pub(crate) fn error_output(json: bool, err: GumgumError) -> String {
    let report = err.to_report();
    if json {
        return serde_json::to_string_pretty(&report).expect("serialize error");
    }
    let mut lines = vec![format!(
        "error: {}",
        report
            .kind
            .map(error_kind_text)
            .unwrap_or_else(|| error_descriptor_text(report.error))
    )];
    if report.kind.is_none() {
        lines.push(format!("detail: {}", report.message));
    }
    if let Some(cause) = report.likely_cause {
        lines.push(format!("cause: {cause}"));
    }
    for command in report.next_commands {
        lines.push(format!("next: {command}"));
    }
    lines.join("\n")
}

fn error_kind_text(kind: gumgum_core::ErrorKind) -> &'static str {
    match kind {
        gumgum_core::ErrorKind::HomeReadFailed => "could not read HOME",
        gumgum_core::ErrorKind::ConfigDirectoryCreateFailed => "could not create config directory",
        gumgum_core::ErrorKind::ConfigReadFailed => "could not read config",
        gumgum_core::ErrorKind::ConfigParseFailed => "could not parse config",
        gumgum_core::ErrorKind::ConfigWriteFailed => "could not write config",
        gumgum_core::ErrorKind::DomainListReadFailed => "could not read domain list",
        gumgum_core::ErrorKind::DomainListParseFailed => "could not parse domain list",
        gumgum_core::ErrorKind::DomainListWriteFailed => "could not write domain list",
        gumgum_core::ErrorKind::ServerListReadFailed => "could not read server list",
        gumgum_core::ErrorKind::ServerListParseFailed => "could not parse server list",
        gumgum_core::ErrorKind::ServerListWriteFailed => "could not write server list",
        gumgum_core::ErrorKind::CloudflareGrantReadFailed => "could not read Cloudflare grant",
        gumgum_core::ErrorKind::CloudflareGrantParseFailed => "could not parse Cloudflare grant",
        gumgum_core::ErrorKind::CloudflareGrantWriteFailed => "could not write Cloudflare grant",
        gumgum_core::ErrorKind::ProviderConfigReadFailed => "could not read provider config",
        gumgum_core::ErrorKind::ProviderConfigParseFailed => "could not parse provider config",
        gumgum_core::ErrorKind::ProviderConfigWriteFailed => "could not write provider config",
        gumgum_core::ErrorKind::ProviderCredentialsReadFailed => {
            "could not read provider credentials"
        }
        gumgum_core::ErrorKind::ProviderCredentialsParseFailed => {
            "could not parse provider credentials"
        }
        gumgum_core::ErrorKind::ProviderCredentialsWriteFailed => {
            "could not write provider credentials"
        }
        gumgum_core::ErrorKind::GraphDirectoryCreateFailed => "could not create graph directory",
        gumgum_core::ErrorKind::GraphDatabaseUrlBuildFailed => "could not build graph database URL",
        gumgum_core::ErrorKind::GraphDatabaseOpenFailed => {
            "could not open graph database for migrations"
        }
        gumgum_core::ErrorKind::GraphDatabaseMigrationFailed => {
            "could not run graph database migrations"
        }
        gumgum_core::ErrorKind::SetupCommandSpawnFailed => "failed to run setup command",
        gumgum_core::ErrorKind::SetupCommandFailed => "setup command failed",
        gumgum_core::ErrorKind::GraphValueInvalid => "graph value is invalid",
        gumgum_core::ErrorKind::ControlPlaneEventKindUnknown => "unknown control plane event kind",
        gumgum_core::ErrorKind::ReconcileEventStatusUnknown => {
            "unknown reconciliation event status"
        }
        gumgum_core::ErrorKind::ManifestReadFailed => "could not read manifest",
        gumgum_core::ErrorKind::ManifestParseFailed => "could not parse manifest",
        gumgum_core::ErrorKind::ManifestValidationFailed => "manifest validation failed",
        gumgum_core::ErrorKind::HttpClientBuildFailed => "failed to build HTTP client",
        gumgum_core::ErrorKind::DaemonReachFailed => "failed to reach gumgumd",
        gumgum_core::ErrorKind::DaemonReturnedError => "gumgumd returned an error",
        gumgum_core::ErrorKind::DaemonInvalidJson => "gumgumd returned invalid JSON",
        gumgum_core::ErrorKind::DockerDaemonRequestFailed => "Docker daemon request failed",
        gumgum_core::ErrorKind::DockerExecFailed => "Docker exec failed",
        gumgum_core::ErrorKind::DnsmasqConfigWriteFailed => "could not write dnsmasq config",
        gumgum_core::ErrorKind::DnsmasqConfigDirectoryCreateFailed => {
            "could not create dnsmasq config directory"
        }
        gumgum_core::ErrorKind::DeploymentContainerHealthCheckFailed => {
            "deployment container did not become healthy"
        }
        gumgum_core::ErrorKind::GraphExecutionInjectedFailure => "injected graph execution failure",
        gumgum_core::ErrorKind::CloudflareZoneNotFound => "Cloudflare zone was not found",
        gumgum_core::ErrorKind::CloudflareTunnelCreateResponseDecodeFailed => {
            "could not decode Cloudflare tunnel create response"
        }
        gumgum_core::ErrorKind::CloudflareTunnelTokenResponseDecodeFailed => {
            "could not decode Cloudflare tunnel token response"
        }
        gumgum_core::ErrorKind::CloudflareApiRequestFailed => "Cloudflare API request failed",
        gumgum_core::ErrorKind::CloudflareApiReturnedError => "Cloudflare API returned an error",
        gumgum_core::ErrorKind::CloudflareApiResponseBodyReadFailed => {
            "could not read Cloudflare API response body"
        }
        gumgum_core::ErrorKind::CloudflareApiResponseDecodeFailed => {
            "could not decode Cloudflare API response"
        }
        gumgum_core::ErrorKind::CloudflareApiResultMissing => {
            "Cloudflare API response did not include a result"
        }
        gumgum_core::ErrorKind::CloudflareTokenRequired => "Cloudflare API token required",
        gumgum_core::ErrorKind::CloudflareTokenEmpty => "Cloudflare token cannot be empty",
        gumgum_core::ErrorKind::PublishedRouteDomainNotManaged => {
            "no managed domain matches published route"
        }
    }
}

fn error_descriptor_text(error: gumgum_core::ErrorDescriptor) -> &'static str {
    match (error.subsystem, error.code) {
        (gumgum_core::Subsystem::Cli, gumgum_core::ErrorCode::InvalidArgs) => {
            "invalid command arguments"
        }
        (gumgum_core::Subsystem::Manifest, gumgum_core::ErrorCode::ManifestNotFound) => {
            "manifest was not found"
        }
        (gumgum_core::Subsystem::Manifest, gumgum_core::ErrorCode::ManifestParseFailed) => {
            "manifest could not be parsed"
        }
        (gumgum_core::Subsystem::Manifest, gumgum_core::ErrorCode::ManifestValidationFailed) => {
            "manifest is invalid"
        }
        (_, gumgum_core::ErrorCode::Io) => "I/O operation failed",
        (_, gumgum_core::ErrorCode::NotImplemented) => "operation is not implemented",
        (_, gumgum_core::ErrorCode::InvalidArgs) => "invalid arguments",
        (_, gumgum_core::ErrorCode::ManifestNotFound) => "manifest was not found",
        (_, gumgum_core::ErrorCode::ManifestParseFailed) => "manifest could not be parsed",
        (_, gumgum_core::ErrorCode::ManifestValidationFailed) => "manifest is invalid",
    }
}

pub(crate) fn print_error(json: bool, err: GumgumError) {
    let output = error_output(json, err);
    if json {
        println!("{output}");
    } else {
        eprintln!("{output}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{ErrorCode, ErrorKind, Subsystem};

    fn sample_error() -> GumgumError {
        GumgumError::structured(Subsystem::Cli, ErrorCode::InvalidArgs, "bad input")
            .likely_cause("missing --host")
            .next_command("gumgum server add <host>")
            .build()
    }

    #[test]
    fn human_errors_are_plain_lines_with_next_steps() {
        let output = error_output(false, sample_error());
        assert!(output.contains("error: invalid command arguments"));
        assert!(output.contains("detail: bad input"));
        assert!(output.contains("cause: missing --host"));
        assert!(output.contains("next: gumgum server add <host>"));
        assert!(!output.trim_start().starts_with('{'));
    }

    #[test]
    fn human_errors_render_known_kinds_without_core_detail_text() {
        let output = error_output(
            false,
            GumgumError::structured_kind(
                Subsystem::Config,
                ErrorCode::Io,
                ErrorKind::ConfigReadFailed,
            )
            .likely_cause("permission denied")
            .build(),
        );
        assert!(output.contains("error: could not read config"));
        assert!(output.contains("cause: permission denied"));
        assert!(!output.contains("detail:"));
    }

    #[test]
    fn json_errors_are_structured_reports() {
        let output = error_output(true, sample_error());
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["error"]["subsystem"], "cli");
        assert_eq!(value["error"]["code"], "INVALID_ARGS");
        assert!(value.get("kind").is_none());
        assert_eq!(value["message"], "bad input");
        assert_eq!(value["likely_cause"], "missing --host");
        assert_eq!(value["next_commands"][0], "gumgum server add <host>");
    }
}
