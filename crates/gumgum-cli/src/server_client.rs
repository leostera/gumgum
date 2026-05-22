use gumgum_api::{
    AffectedReport, BindingDeleteRequest, BindingReport, BindingRequest, BucketObjectReport,
    BucketObjectRequest, DaemonVersionReport, DeployApplyReport, DeployRequest,
    DeploymentDeleteRequest, DeploymentRevisionsReport, DomainAddRequest, DomainReport, EnvReport,
    EventsReport, GraphReport, LogsReport, ObjectDeleteRequest, ObjectReport, ObjectRequest,
    ProviderBootReport, ProviderStatusReport, RollbackReport, RollbackRequest,
};
use gumgum_core::{ErrorCode, GumgumError, Subsystem};

#[derive(Clone)]
pub(crate) struct ServerClient {
    host: String,
    http: reqwest::Client,
}

impl ServerClient {
    pub(crate) fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            http: reqwest::Client::new(),
        }
    }

    pub(crate) async fn version(&self) -> gumgum_core::Result<DaemonVersionReport> {
        self.get_json("/v0/version", "version").await
    }

    pub(crate) async fn graph(&self) -> gumgum_core::Result<GraphReport> {
        self.get_json("/v0/graph", "graph").await
    }

    pub(crate) async fn affected(&self, target: &str) -> gumgum_core::Result<AffectedReport> {
        let url =
            reqwest::Url::parse_with_params(&self.url("/v0/graph/affected"), &[("target", target)])
                .map_err(|source| {
                    GumgumError::structured(
                        Subsystem::Api,
                        ErrorCode::InvalidArgs,
                        "invalid graph affected URL",
                    )
                    .likely_cause(source.to_string())
                    .build()
                })?;
        self.http
            .get(url)
            .send()
            .await
            .map_err(|source| self.api_error("failed to call gumgumd graph affected API", source))?
            .error_for_status()
            .map_err(|source| {
                self.api_error("gumgumd graph affected API returned an error", source)
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error("gumgumd graph affected API returned invalid JSON", source)
            })
    }

    pub(crate) async fn create_object(
        &self,
        request: &ObjectRequest,
    ) -> gumgum_core::Result<ObjectReport> {
        self.post_json("/v0/objects", request, "object").await
    }

    pub(crate) async fn delete_object(
        &self,
        request: &ObjectDeleteRequest,
    ) -> gumgum_core::Result<ObjectReport> {
        self.delete_json("/v0/objects", request, "object delete")
            .await
    }

    pub(crate) async fn add_domain(
        &self,
        request: &DomainAddRequest,
    ) -> gumgum_core::Result<DomainReport> {
        self.post_json("/v0/domains", request, "domain").await
    }

    pub(crate) async fn providers(&self) -> gumgum_core::Result<ProviderStatusReport> {
        self.get_json("/v0/providers", "providers").await
    }

    pub(crate) async fn boot_default_providers(&self) -> gumgum_core::Result<ProviderBootReport> {
        self.post_json(
            "/v0/providers/defaults/boot",
            &serde_json::json!({}),
            "provider boot",
        )
        .await
    }

    pub(crate) async fn bind_object(
        &self,
        request: &BindingRequest,
    ) -> gumgum_core::Result<BindingReport> {
        self.post_json("/v0/bindings", request, "binding").await
    }

    pub(crate) async fn delete_binding(
        &self,
        request: &BindingDeleteRequest,
    ) -> gumgum_core::Result<BindingReport> {
        self.delete_json("/v0/bindings", request, "binding delete")
            .await
    }

    pub(crate) async fn env(&self, worker: &str) -> gumgum_core::Result<EnvReport> {
        self.get_json(&format!("/v0/env/{worker}"), "env").await
    }

    pub(crate) async fn deploy(
        &self,
        request: &DeployRequest,
    ) -> gumgum_core::Result<DeployApplyReport> {
        self.post_json("/v0/deploy", request, "deploy").await
    }

    pub(crate) async fn delete_deploy(
        &self,
        request: &DeploymentDeleteRequest,
    ) -> gumgum_core::Result<DeployApplyReport> {
        self.delete_json("/v0/deploy", request, "deployment delete")
            .await
    }

    pub(crate) async fn rollback(
        &self,
        worker: String,
        preview: bool,
        revision_id: Option<i64>,
    ) -> gumgum_core::Result<RollbackReport> {
        self.post_json(
            "/v0/rollback",
            &RollbackRequest {
                worker,
                preview,
                revision_id,
            },
            "rollback",
        )
        .await
    }

    pub(crate) async fn delete_revision(
        &self,
        worker: &str,
        revision_id: i64,
    ) -> gumgum_core::Result<gumgum_api::DeploymentRevisionDeleteReport> {
        let path = format!("/v0/revisions/{worker}/{revision_id}");
        let response = self
            .http
            .delete(self.url(&path))
            .send()
            .await
            .map_err(|source| {
                self.api_error(
                    "failed to call gumgumd deployment revision delete API",
                    source,
                )
            })?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
        ) {
            return Err(self.unsupported_revision_delete_error(worker, revision_id));
        }
        response
            .error_for_status()
            .map_err(|source| {
                self.api_error(
                    "gumgumd deployment revision delete API returned an error",
                    source,
                )
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error(
                    "gumgumd deployment revision delete API returned invalid JSON",
                    source,
                )
            })
    }

    pub(crate) async fn revisions(
        &self,
        worker: &str,
        limit: u32,
    ) -> gumgum_core::Result<DeploymentRevisionsReport> {
        let path = format!("/v0/revisions/{worker}?limit={limit}");
        let response = self
            .http
            .get(self.url(&path))
            .send()
            .await
            .map_err(|source| self.api_error("failed to call gumgumd revisions API", source))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(self.unsupported_revisions_error(worker));
        }
        response
            .error_for_status()
            .map_err(|source| self.api_error("gumgumd revisions API returned an error", source))?
            .json()
            .await
            .map_err(|source| self.api_error("gumgumd revisions API returned invalid JSON", source))
    }

    pub(crate) async fn logs(&self, container: &str, tail: u32) -> gumgum_core::Result<LogsReport> {
        self.get_json(&format!("/v0/logs/{container}?tail={tail}"), "logs")
            .await
    }

    pub(crate) async fn events(&self, limit: u32) -> gumgum_core::Result<EventsReport> {
        self.get_json(&format!("/v0/events?limit={limit}"), "events")
            .await
    }

    pub(crate) async fn bucket_object(
        &self,
        action: &str,
        request: &BucketObjectRequest,
    ) -> gumgum_core::Result<BucketObjectReport> {
        let path = format!("/v0/buckets/{action}");
        let response = self
            .http
            .post(self.url(&path))
            .json(request)
            .send()
            .await
            .map_err(|source| self.api_error("failed to call gumgumd bucket object API", source))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(self.unsupported_bucket_object_error(action));
        }
        response
            .error_for_status()
            .map_err(|source| {
                self.api_error("gumgumd bucket object API returned an error", source)
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error("gumgumd bucket object API returned invalid JSON", source)
            })
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        name: &str,
    ) -> gumgum_core::Result<T> {
        self.http
            .get(self.url(path))
            .send()
            .await
            .map_err(|source| self.api_error(format!("failed to call gumgumd {name} API"), source))?
            .error_for_status()
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned an error"), source)
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned invalid JSON"), source)
            })
    }

    async fn post_json<T: serde::Serialize + ?Sized, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        request: &T,
        name: &str,
    ) -> gumgum_core::Result<R> {
        self.http
            .post(self.url(path))
            .json(request)
            .send()
            .await
            .map_err(|source| self.api_error(format!("failed to call gumgumd {name} API"), source))?
            .error_for_status()
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned an error"), source)
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned invalid JSON"), source)
            })
    }

    async fn delete_json<T: serde::Serialize + ?Sized, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        request: &T,
        name: &str,
    ) -> gumgum_core::Result<R> {
        let response = self
            .http
            .delete(self.url(path))
            .json(request)
            .send()
            .await
            .map_err(|source| {
                self.api_error(format!("failed to call gumgumd {name} API"), source)
            })?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(self.unsupported_delete_error(path, name));
        }
        response
            .error_for_status()
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned an error"), source)
            })?
            .json()
            .await
            .map_err(|source| {
                self.api_error(format!("gumgumd {name} API returned invalid JSON"), source)
            })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}:7777{}", self.host, path)
    }

    fn api_error(&self, message: impl Into<String>, source: impl ToString) -> GumgumError {
        GumgumError::structured(Subsystem::Api, ErrorCode::Io, message.into())
            .likely_cause(source.to_string())
            .build()
    }

    fn unsupported_delete_error(&self, path: &str, name: &str) -> GumgumError {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            format!("gumgumd does not expose safe {name} API"),
        )
        .likely_cause(format!(
            "server {} returned 405 for DELETE {path}; the daemon is probably older than the CLI or was installed before safe delete/unbind APIs were added",
            self.host
        ))
        .next_command(format!(
            "gumgum server add {} --domain <domain>",
            self.host
        ))
        .next_command(format!("gumgum server upgrade --host {}", self.host))
        .build()
    }

    fn unsupported_revision_delete_error(&self, worker: &str, revision_id: i64) -> GumgumError {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd does not expose safe deployment revision delete API",
        )
        .likely_cause(format!(
            "server {} returned 404/405 for DELETE /v0/revisions/{worker}/{revision_id}; the daemon is probably older than the CLI or was installed before safe rollback revision pruning was added",
            self.host
        ))
        .next_command(format!(
            "gumgum server add {} --domain <domain>",
            self.host
        ))
        .next_command(format!("gumgum server upgrade --host {}", self.host))
        .build()
    }

    fn unsupported_revisions_error(&self, worker: &str) -> GumgumError {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd does not expose deployment revisions",
        )
        .likely_cause(format!(
            "server {} returned 404 for /v0/revisions/{worker}; the daemon is probably older than the CLI or was installed from a release before rollback revisions were added",
            self.host
        ))
        .next_command(format!(
            "gumgum server add {} --domain <domain>",
            self.host
        ))
        .next_command(format!("gumgum server upgrade --host {}", self.host))
        .build()
    }

    fn unsupported_bucket_object_error(&self, action: &str) -> GumgumError {
        GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "gumgumd does not expose bucket object API",
        )
        .likely_cause(format!(
            "server {} returned 404 for POST /v0/buckets/{action}; the daemon is probably older than the CLI or was installed before bucket object APIs were added",
            self.host
        ))
        .next_command(format!("gumgum server upgrade --host {}", self.host))
        .next_command(format!(
            "gumgum server capabilities list --host {} --require gumgum:buckets:objects",
            self.host
        ))
        .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_delete_error_explains_old_daemon() {
        let report = ServerClient::new("starbase2")
            .unsupported_delete_error("/v0/bindings", "binding delete")
            .to_report();

        assert_eq!(
            report.message,
            "gumgumd does not expose safe binding delete API"
        );
        assert!(report.likely_cause.unwrap().contains("DELETE /v0/bindings"));
        assert_eq!(
            report.next_commands,
            vec![
                "gumgum server add starbase2 --domain <domain>",
                "gumgum server upgrade --host starbase2",
            ]
        );
    }

    #[test]
    fn unsupported_revision_delete_error_explains_old_daemon() {
        let report = ServerClient::new("starbase2")
            .unsupported_revision_delete_error("api", 8)
            .to_report();

        assert_eq!(
            report.message,
            "gumgumd does not expose safe deployment revision delete API"
        );
        assert!(
            report
                .likely_cause
                .unwrap()
                .contains("DELETE /v0/revisions/api/8")
        );
        assert_eq!(
            report.next_commands,
            vec![
                "gumgum server add starbase2 --domain <domain>",
                "gumgum server upgrade --host starbase2",
            ]
        );
    }

    #[test]
    fn unsupported_revisions_error_explains_old_daemon() {
        let report = ServerClient::new("192.168.0.3")
            .unsupported_revisions_error("hello-world")
            .to_report();

        assert_eq!(
            report.message,
            "gumgumd does not expose deployment revisions"
        );
        assert!(
            report
                .likely_cause
                .unwrap()
                .contains("/v0/revisions/hello-world")
        );
        assert_eq!(
            report.next_commands,
            vec![
                "gumgum server add 192.168.0.3 --domain <domain>",
                "gumgum server upgrade --host 192.168.0.3",
            ]
        );
    }

    #[test]
    fn unsupported_bucket_object_error_explains_old_daemon() {
        let report = ServerClient::new("starbase2")
            .unsupported_bucket_object_error("ls")
            .to_report();

        assert_eq!(report.message, "gumgumd does not expose bucket object API");
        assert!(report.likely_cause.unwrap().contains("POST /v0/buckets/ls"));
        assert_eq!(
            report.next_commands,
            vec![
                "gumgum server upgrade --host starbase2",
                "gumgum server capabilities list --host starbase2 --require gumgum:buckets:objects",
            ]
        );
    }
}
