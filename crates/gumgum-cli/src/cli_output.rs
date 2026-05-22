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

pub(crate) fn print_error(json: bool, err: GumgumError) {
    let report = err.to_report();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("serialize error")
        );
        return;
    }
    eprintln!("error: {}", report.message);
    if let Some(cause) = report.likely_cause {
        eprintln!("cause: {cause}");
    }
    for command in report.next_commands {
        eprintln!("next: {command}");
    }
}
