use crate::{EnvArgs, print_value, resolve_server};
use gumgum_api::EnvReport;
use gumgum_core::load_worker_path;

use crate::server_client::ServerClient;

pub(crate) async fn env(args: EnvArgs, json: bool) -> gumgum_core::Result<()> {
    let worker = args.worker.unwrap_or_else(|| {
        load_worker_path(&args.path)
            .map(|manifest| manifest.worker.name)
            .unwrap_or_else(|_| "unknown".to_owned())
    });
    let server = resolve_server(args.host)?;
    let report = ServerClient::new(server.host).env(&worker).await?;
    if json {
        print_value(true, &report);
    } else {
        for line in dotenv_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn dotenv_lines(report: &EnvReport) -> Vec<String> {
    report
        .vars
        .iter()
        .map(|var| format!("{}={}", var.name, dotenv_quote(&var.value)))
        .collect()
}

fn dotenv_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_api::EnvVar;

    #[test]
    fn dotenv_lines_quote_only_when_needed() {
        let report = EnvReport {
            ok: true,
            worker: "api".to_owned(),
            vars: vec![
                EnvVar {
                    name: "DATABASE_URL".to_owned(),
                    value: "postgres://api:g@db.example:5432/api".to_owned(),
                },
                EnvVar {
                    name: "GREETING".to_owned(),
                    value: "hello world".to_owned(),
                },
            ],
            message: "2 environment variable(s)".to_owned(),
        };

        assert_eq!(
            dotenv_lines(&report),
            vec![
                "DATABASE_URL=postgres://api:g@db.example:5432/api",
                "GREETING='hello world'",
            ]
        );
    }
}
