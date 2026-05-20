use gumgum_api::{
    AffectedReport, BindingReport, BindingRequest, DeployApplyReport, DeployRequest, GraphReport,
    LogsReport, ObjectReport, ObjectRequest, RollbackReport, RollbackRequest,
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

    pub(crate) async fn bind_object(
        &self,
        request: &BindingRequest,
    ) -> gumgum_core::Result<BindingReport> {
        self.post_json("/v0/bindings", request, "binding").await
    }

    pub(crate) async fn deploy(
        &self,
        request: &DeployRequest,
    ) -> gumgum_core::Result<DeployApplyReport> {
        self.post_json("/v0/deploy", request, "deploy").await
    }

    pub(crate) async fn rollback(
        &self,
        worker: String,
        preview: bool,
    ) -> gumgum_core::Result<RollbackReport> {
        self.post_json(
            "/v0/rollback",
            &RollbackRequest { worker, preview },
            "rollback",
        )
        .await
    }

    pub(crate) async fn logs(&self, container: &str, tail: u32) -> gumgum_core::Result<LogsReport> {
        self.get_json(&format!("/v0/logs/{container}?tail={tail}"), "logs")
            .await
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

    fn url(&self, path: &str) -> String {
        format!("http://{}:7777{}", self.host, path)
    }

    fn api_error(&self, message: impl Into<String>, source: impl ToString) -> GumgumError {
        GumgumError::structured(Subsystem::Api, ErrorCode::Io, message.into())
            .likely_cause(source.to_string())
            .build()
    }
}
