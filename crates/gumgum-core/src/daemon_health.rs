use crate::{ErrorCode, ErrorKind, GumgumError, Subsystem};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonPingReport {
    pub ok: bool,
    pub host: String,
    pub health_url: String,
    pub service_active: Option<bool>,
    pub health: serde_json::Value,
}

pub struct DaemonHealthClient;

impl DaemonHealthClient {
    pub async fn ping(host: &str) -> crate::Result<DaemonPingReport> {
        let health_url = format!("http://{host}:7777/healthz");
        let health: serde_json::Value = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|source| {
                GumgumError::structured_kind(
                    Subsystem::Api,
                    ErrorCode::Io,
                    ErrorKind::HttpClientBuildFailed,
                )
                .likely_cause(source.to_string())
                .build()
            })?
            .get(&health_url)
            .send()
            .await
            .map_err(|source| {
                GumgumError::structured_kind(
                    Subsystem::Api,
                    ErrorCode::Io,
                    ErrorKind::DaemonReachFailed,
                )
                .likely_cause(source.to_string())
                .next_command(format!("gumgum setup {host} --domain <domain>"))
                .build()
            })?
            .error_for_status()
            .map_err(|source| {
                GumgumError::structured_kind(
                    Subsystem::Api,
                    ErrorCode::Io,
                    ErrorKind::DaemonReturnedError,
                )
                .likely_cause(source.to_string())
                .build()
            })?
            .json()
            .await
            .map_err(|source| {
                GumgumError::structured_kind(
                    Subsystem::Api,
                    ErrorCode::Io,
                    ErrorKind::DaemonInvalidJson,
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        let ok = health
            .get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        Ok(DaemonPingReport {
            ok,
            host: host.to_owned(),
            health_url,
            service_active: Some(ok),
            health,
        })
    }

    pub async fn wait_for_ping(host: &str) -> crate::Result<DaemonPingReport> {
        let mut last_error = None;
        for _ in 0..120 {
            match Self::ping(host).await {
                Ok(report) if report.ok => return Ok(report),
                Ok(report) => {
                    last_error = Some(format!("health_ok={}", report.ok));
                }
                Err(err) => {
                    let report = err.to_report();
                    last_error = Some(report.likely_cause.unwrap_or(report.message));
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(GumgumError::structured_kind(
            Subsystem::Api,
            ErrorCode::Io,
            ErrorKind::DaemonReachFailed,
        )
        .likely_cause(last_error.unwrap_or_else(|| "health_check_timeout".to_owned()))
        .next_command(format!("gumgum setup {host} --domain <domain>"))
        .build())
    }
}
