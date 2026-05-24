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
    let mut lines = vec![format!("error: {}", error_descriptor_text(report.error))];
    lines.push(format!("detail: {}", report.message));
    if let Some(cause) = report.likely_cause {
        lines.push(format!("cause: {cause}"));
    }
    for command in report.next_commands {
        lines.push(format!("next: {command}"));
    }
    lines.join("\n")
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
    use gumgum_core::{ErrorCode, Subsystem};

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
    fn json_errors_are_structured_reports() {
        let output = error_output(true, sample_error());
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["error"]["subsystem"], "cli");
        assert_eq!(value["error"]["code"], "INVALID_ARGS");
        assert_eq!(value["message"], "bad input");
        assert_eq!(value["likely_cause"], "missing --host");
        assert_eq!(value["next_commands"][0], "gumgum server add <host>");
    }
}
