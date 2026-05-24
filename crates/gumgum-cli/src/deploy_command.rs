#![allow(clippy::items_after_test_module)]

use crate::{DeployArgs, event_presenter::print_events_json_lines, progress, resolve_server};
use gumgum_api::{
    DeployApplyReport, DeployRequest, DeploymentDeleteRequest, GrafanaArtifactRequest,
    PrometheusScrapeRequest, ServerRecord,
};
use gumgum_core::{
    ConfigStore, DeploymentDescriptor, ErrorCode, GumgumError, GumgumEvent, ManifestKind,
    PlanGraph, Subsystem, WorkerManifest, load_worker_path, load_workspace_path,
    run_setup_command_streaming as run_command_streaming, validate_path,
};
use serde::Serialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command as TokioCommand;

use crate::{deploy_executor::DeployExecutor, server_client::ServerClient};

#[derive(Debug, Serialize)]
pub(crate) struct GrafanaArtifactPlan {
    pub(crate) kind: String,
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) folder_path: Vec<String>,
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DeployReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) path: String,
    pub(crate) worker: String,
    pub(crate) host: Option<String>,
    pub(crate) build_context: Option<String>,
    pub(crate) image: String,
    pub(crate) container: String,
    pub(crate) port: u16,
    pub(crate) routes: Vec<String>,
    pub(crate) health_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) grafana: Vec<GrafanaArtifactPlan>,
    pub(crate) plan: Vec<String>,
    pub(crate) plan_graph: PlanGraph,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) events: Vec<GumgumEvent>,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceDeployReport {
    pub(crate) ok: bool,
    pub(crate) dry_run: bool,
    pub(crate) path: String,
    pub(crate) workspace: String,
    pub(crate) workers: Vec<DeployReport>,
    pub(crate) plan: Vec<String>,
    pub(crate) message: String,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum DeployOutput {
    Worker(DeployReport),
    Workspace(WorkspaceDeployReport),
    Delete(DeployApplyReport),
}

pub(crate) async fn deploy(
    args: DeployArgs,
    dry_run: bool,
    quiet: bool,
) -> gumgum_core::Result<DeployOutput> {
    let kind = validate_path(&args.path)?.manifest_kind;
    match kind {
        ManifestKind::Worker => {
            let manifest = load_worker_path(&args.path)?;
            let server = resolve_deploy_server(args.host.clone())?;
            if args.delete {
                let Some(server) = server else {
                    return Err(GumgumError::structured(
                        Subsystem::Config,
                        ErrorCode::InvalidArgs,
                        "no gumgum server configured",
                    )
                    .next_command("gumgum server list")
                    .build());
                };
                let report = ServerClient::new(server.host)
                    .delete_deploy(&DeploymentDeleteRequest {
                        worker: deployment_key(&manifest.worker.name, args.env),
                        preview: dry_run,
                    })
                    .await?;
                return Ok(DeployOutput::Delete(report));
            }
            let namespace = manifest
                .project
                .as_ref()
                .map(|project| project.namespace.as_str());
            let report = deploy_one(
                args.path.clone(),
                &manifest,
                DeployProjectContext {
                    name: namespace,
                    domain: None,
                },
                server,
                dry_run,
                args.env,
                quiet,
            )
            .await?;
            Ok(DeployOutput::Worker(report))
        }
        ManifestKind::Workspace => {
            let workspace = load_workspace_path(&args.path)?;
            let server = resolve_deploy_server(
                args.host
                    .clone()
                    .or_else(|| workspace.server().map(ToOwned::to_owned)),
            )?;
            let root = args
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let mut workers = Vec::new();
            let mut plan = vec![format!("project {}", workspace.project_name())];
            for member in workspace.members() {
                let member_path = root.join(member).join("gumgum.toml");
                let manifest = load_worker_path(&member_path)?;
                let report = deploy_one(
                    member_path,
                    &manifest,
                    DeployProjectContext {
                        name: Some(workspace.project_name()),
                        domain: Some(workspace.domain()),
                    },
                    server.clone(),
                    dry_run,
                    args.env,
                    quiet,
                )
                .await?;
                plan.extend(
                    report
                        .plan
                        .iter()
                        .map(|step| format!("{}: {step}", report.worker)),
                );
                workers.push(report);
            }
            Ok(DeployOutput::Workspace(WorkspaceDeployReport {
                ok: true,
                dry_run,
                path: args.path.display().to_string(),
                workspace: workspace.project_name().to_owned(),
                workers,
                plan,
                message: if dry_run {
                    "workspace deploy plan"
                } else {
                    "workspace deployed"
                }
                .to_owned(),
            }))
        }
    }
}

fn resolve_deploy_server(host: Option<String>) -> gumgum_core::Result<Option<ServerRecord>> {
    match host {
        Some(host) => Ok(Some(resolve_server(Some(host))?)),
        None => ConfigStore::from_home_env().and_then(|store| store.load_default_server()),
    }
}

#[derive(Clone, Copy)]
struct DeployProjectContext<'a> {
    name: Option<&'a str>,
    domain: Option<&'a str>,
}

async fn deploy_one(
    path: PathBuf,
    manifest: &WorkerManifest,
    project: DeployProjectContext<'_>,
    server: Option<ServerRecord>,
    dry_run: bool,
    env: crate::DeployEnv,
    quiet: bool,
) -> gumgum_core::Result<DeployReport> {
    let mut report = deploy_report(
        path,
        manifest,
        project.name,
        project.domain,
        server.as_ref(),
        dry_run,
        env,
    );
    if dry_run {
        return Ok(report);
    }
    let server = server.ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            "no GumGum.dev server configured",
        )
        .next_command("gumgum setup <host> --domain <domain>")
        .build()
    })?;
    DeployExecutor::new(&server, quiet)
        .ensure_manifest_bindings(manifest, project.name, env)
        .await?;
    report.events.push(GumgumEvent::DeploymentStarted {
        worker: report.worker.clone(),
        environment: Some(env.label().to_owned()),
        image: report.image.clone(),
    });
    run_remote_deploy(&server, manifest, &report, env, quiet).await?;
    report.ok = true;
    report.dry_run = false;
    report.message = match &report.health_url {
        Some(health_url) => format!(
            "deployed {} to {}; health verified at {}",
            report.worker, server.host, health_url
        ),
        None => format!("deployed {} to {}", report.worker, server.host),
    };
    report.events.push(GumgumEvent::DeploymentSucceeded {
        worker: report.worker.clone(),
        environment: Some(env.label().to_owned()),
        revision: Some(
            report
                .image
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .to_owned(),
        ),
        route: report.routes.first().cloned(),
    });
    Ok(report)
}

fn deploy_report(
    path: PathBuf,
    manifest: &WorkerManifest,
    project_name: Option<&str>,
    project_domain: Option<&str>,
    server: Option<&ServerRecord>,
    dry_run: bool,
    env: crate::DeployEnv,
) -> DeployReport {
    let mut descriptor = DeploymentDescriptor::from_manifest_in_project(
        &path,
        manifest,
        project_name,
        project_domain,
        server,
        env.is_prod(),
    );
    descriptor.container = env_prefixed_container_name(&descriptor.container, env);
    let grafana = grafana_artifact_plan(&path, manifest, project_name, project_domain);
    descriptor.plan.extend(grafana.iter().map(|artifact| {
        format!(
            "provision Grafana {} {} from {}",
            artifact.kind, artifact.name, artifact.path
        )
    }));
    let events = vec![GumgumEvent::DeploymentPlanned {
        worker: descriptor.worker.clone(),
        environment: Some(env.label().to_owned()),
        image: descriptor.image.clone(),
        route: descriptor.routes.first().cloned(),
    }];
    DeployReport {
        ok: true,
        dry_run,
        path: path.display().to_string(),
        worker: descriptor.worker,
        host: server.map(|server| server.host.clone()),
        build_context: descriptor.build_context,
        image: descriptor.image,
        container: descriptor.container,
        port: descriptor.port,
        routes: descriptor.routes,
        health_url: descriptor.health_url,
        grafana,
        plan: descriptor.plan,
        plan_graph: descriptor.plan_graph,
        events,
        message: if dry_run {
            format!(
                "validated worker manifest for {} deploy; no containers changed",
                env.label()
            )
        } else {
            "deployment pending".to_owned()
        },
    }
}

async fn run_remote_deploy(
    server: &ServerRecord,
    manifest: &WorkerManifest,
    report: &DeployReport,
    env: crate::DeployEnv,
    quiet: bool,
) -> gumgum_core::Result<()> {
    let context = report
        .build_context
        .as_deref()
        .unwrap_or_else(|| manifest.worker.build_context.as_deref().unwrap_or("."));
    let host = &server.host;
    let tunnel_port = 55001;
    let local_image = local_push_image(&report.image, tunnel_port);
    let route = deploy_route(report, server);

    wait_for_remote_registry(host, quiet).await?;
    progress(quiet, format!("opening registry tunnel to {host}"));
    let mut tunnel = TokioCommand::new("ssh")
        .arg("-N")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-L")
        .arg(format!("{tunnel_port}:127.0.0.1:55000"))
        .arg(host)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Setup,
                ErrorCode::Io,
                "could not open registry tunnel",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    progress(quiet, format!("building image {local_image} locally"));
    let build_result = run_command_streaming(
        TokioCommand::new("docker")
            .arg("build")
            .arg("--no-cache")
            .arg("--platform")
            .arg("linux/amd64")
            .arg("-t")
            .arg(&local_image)
            .arg(context),
        quiet,
    )
    .await;
    let push_result = if build_result.is_ok() {
        progress(quiet, "pushing image to GumGum.dev registry");
        Some(
            run_command_streaming(
                TokioCommand::new("docker").arg("push").arg(&local_image),
                quiet,
            )
            .await,
        )
    } else {
        None
    };
    let _ = tunnel.kill().await;
    if let Err(error) = build_result {
        return Err(deploy_command_failure(
            "Docker image build failed",
            error,
            "docker info",
        ));
    }
    if let Some(Err(error)) = push_result {
        return Err(deploy_command_failure(
            "Docker image push to GumGum.dev registry failed",
            error,
            format!("gumgum status --host {host}"),
        ));
    }

    progress(
        quiet,
        format!("asking gumgumd on {host} to reconcile {}", report.worker),
    );
    let request = DeployRequest {
        worker: deployment_key(&report.worker, env),
        image: report.image.clone(),
        container: report.container.clone(),
        route: route.clone(),
        publish: manifest.ingress.iter().any(|ingress| ingress.public),
        port: report.port,
        health: manifest.worker.ready_check_path().to_owned(),
    };
    apply_deploy_via_daemon(host, &request).await?;
    if let Some(metrics_path) = observability_metrics_path(manifest) {
        configure_prometheus_scrape(host, report, env, metrics_path, quiet).await?;
    }
    apply_grafana_artifacts(host, &report.grafana, quiet).await?;
    if manifest.ingress.is_empty() {
        progress(
            quiet,
            format!(
                "{} has no ingress; container health was verified by gumgumd",
                report.worker
            ),
        );
        Ok(())
    } else {
        let route = route.as_ref().expect("ingress deploy has a route");
        verify_route(server, route, manifest.worker.ready_check_path(), quiet).await?;
        if let Some(metrics_path) = observability_metrics_path(manifest) {
            verify_observability_metrics(server, route, metrics_path, quiet).await?;
        }
        Ok(())
    }
}

async fn configure_prometheus_scrape(
    host: &str,
    report: &DeployReport,
    env: crate::DeployEnv,
    metrics_path: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    progress(
        quiet,
        format!(
            "configuring Prometheus scrape for {} at {}",
            report.worker, metrics_path
        ),
    );
    let response = ServerClient::new(host.to_owned())
        .configure_prometheus_scrape(&PrometheusScrapeRequest {
            worker: report.worker.clone(),
            environment: env.label().to_owned(),
            container: report.container.clone(),
            port: report.port,
            metrics_path: metrics_path.to_owned(),
        })
        .await?;
    if response.ok {
        Ok(())
    } else {
        Err(GumgumError::structured(Subsystem::Api, ErrorCode::Io, response.message).build())
    }
}

async fn apply_grafana_artifacts(
    host: &str,
    artifacts: &[GrafanaArtifactPlan],
    quiet: bool,
) -> gumgum_core::Result<()> {
    if artifacts.is_empty() {
        return Ok(());
    }
    let client = ServerClient::new(host.to_owned());
    for artifact in artifacts {
        progress(
            quiet,
            format!("applying Grafana {} {}", artifact.kind, artifact.name),
        );
        let content = load_grafana_artifact_content(&artifact.path)?;
        let report = client
            .apply_grafana_artifact(&GrafanaArtifactRequest {
                kind: artifact.kind.clone(),
                name: artifact.name.clone(),
                folder_path: artifact.folder_path.clone(),
                content,
            })
            .await?;
        if !report.ok {
            return Err(
                GumgumError::structured(Subsystem::Api, ErrorCode::Io, report.message).build(),
            );
        }
    }
    Ok(())
}

fn load_grafana_artifact_content(path: &str) -> gumgum_core::Result<serde_json::Value> {
    let bytes = std::fs::read(path).map_err(|error| {
        GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::Io,
            format!("could not read Grafana artifact {path}"),
        )
        .likely_cause(error.to_string())
        .build()
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::ManifestParseFailed,
            format!("could not parse Grafana artifact {path}"),
        )
        .likely_cause(error.to_string())
        .build()
    })
}

async fn apply_deploy_via_daemon(
    host: &str,
    request: &DeployRequest,
) -> gumgum_core::Result<DeployApplyReport> {
    let client = ServerClient::new(host);
    let version = client.version().await.ok();
    let report = if version.as_ref().is_some_and(supports_deploy_event_stream) {
        let typed_events = client.deploy_event_stream(request).await?;
        let ok = !typed_events.iter().any(|event| {
            matches!(
                event,
                GumgumEvent::DeploymentFailed { .. } | GumgumEvent::ReconcileStepFailed { .. }
            )
        });
        DeployApplyReport {
            ok,
            worker: request.worker.clone(),
            materialized: ok,
            changed: true,
            actions: Vec::new(),
            reconciliation_steps: Vec::new(),
            typed_events,
            message: if ok {
                "deployment event stream completed".to_owned()
            } else {
                "deployment event stream reported failure".to_owned()
            },
        }
    } else {
        client.deploy(request).await?
    };
    if report.ok {
        Ok(report)
    } else {
        Err(GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            format!("gumgumd failed to reconcile deployment {}", request.worker),
        )
        .likely_cause(report.actions.join("; "))
        .next_command(format!("gumgum logs {} --host {host}", request.worker))
        .build())
    }
}

fn deploy_command_failure(
    message: impl Into<String>,
    source: GumgumError,
    next_command: impl Into<String>,
) -> GumgumError {
    let report = source.to_report();
    let likely_cause = report.likely_cause.unwrap_or(report.message);
    GumgumError::structured(Subsystem::Setup, ErrorCode::Io, message)
        .likely_cause(likely_cause)
        .next_command(next_command)
        .build()
}

async fn wait_for_remote_registry(host: &str, quiet: bool) -> gumgum_core::Result<()> {
    progress(
        quiet,
        format!("checking GumGum.dev registry managed by daemon on {host}"),
    );
    let script = "for i in $(seq 1 20); do if docker inspect -f '{{.State.Running}}' gumgum-registry 2>/dev/null | grep -q true; then exit 0; fi; sleep 0.5; done; echo 'gumgum-registry is not running; is gumgumd active?' >&2; exit 1";
    run_command_streaming(TokioCommand::new("ssh").arg(host).arg(script), quiet).await
}

fn supports_deploy_event_stream(version: &gumgum_api::DaemonVersionReport) -> bool {
    version
        .capabilities
        .iter()
        .any(|capability| capability == "gumgum:deployments:stream")
}

fn local_push_image(image: &str, tunnel_port: u16) -> String {
    image.replacen("127.0.0.1:55000", &format!("localhost:{tunnel_port}"), 1)
}

fn grafana_artifact_plan(
    manifest_path: &std::path::Path,
    manifest: &WorkerManifest,
    project_name: Option<&str>,
    project_domain: Option<&str>,
) -> Vec<GrafanaArtifactPlan> {
    let base_dir = manifest_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let project = project_name
        .or_else(|| {
            manifest
                .project
                .as_ref()
                .map(|project| project.namespace.as_str())
        })
        .unwrap_or("root");
    let folder_path = grafana_folder_path(project_domain, project);
    let mut artifacts = Vec::new();
    if let Some(sources) = manifest
        .observability
        .as_ref()
        .and_then(|observability| observability.grafana.as_ref())
        .and_then(|grafana| grafana.sources.as_ref())
    {
        artifacts.push(GrafanaArtifactPlan {
            kind: "datasource".to_owned(),
            name: format!("{project} / datasources"),
            folder_path: Vec::new(),
            path: base_dir.join(sources).display().to_string(),
        });
    }
    artifacts.extend(
        manifest
            .dashboards
            .iter()
            .map(|dashboard| GrafanaArtifactPlan {
                kind: "dashboard".to_owned(),
                name: format!("{project} / {}", dashboard.name),
                folder_path: folder_path.clone(),
                path: base_dir.join(&dashboard.path).display().to_string(),
            }),
    );
    artifacts
}

fn grafana_folder_path(project_domain: Option<&str>, project: &str) -> Vec<String> {
    project_domain
        .filter(|domain| !domain.trim().is_empty())
        .map(|domain| vec![domain.to_owned(), project.to_owned()])
        .unwrap_or_else(|| vec![project.to_owned()])
}

fn deployment_key(worker: &str, env: crate::DeployEnv) -> String {
    format!("{worker}@{}", env.label())
}

fn env_prefixed_container_name(container: &str, env: crate::DeployEnv) -> String {
    let suffix = container.strip_prefix("gumgum-").unwrap_or(container);
    format!("gumgum-{}-{suffix}", env.label())
}

fn deploy_route(report: &DeployReport, _server: &ServerRecord) -> Option<String> {
    report.routes.first().cloned()
}

fn observability_metrics_path(manifest: &WorkerManifest) -> Option<&str> {
    manifest
        .observability
        .as_ref()
        .filter(|observability| observability.enable)
        .map(|observability| observability.prometheus_metrics.as_str())
}

async fn verify_observability_metrics(
    server: &ServerRecord,
    route: &str,
    metrics_path: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    progress(
        quiet,
        format!("verifying Prometheus metrics at https://{route}{metrics_path}"),
    );
    verify_route(server, route, metrics_path, quiet)
        .await
        .map_err(|error| {
            let report = error.to_report();
            GumgumError::structured(
                Subsystem::Api,
                ErrorCode::Io,
                "observability metrics endpoint did not respond",
            )
            .likely_cause(report.likely_cause.unwrap_or(report.message))
            .next_command(format!("curl -fsS https://{route}{metrics_path}"))
            .build()
        })
}

async fn verify_route(
    server: &ServerRecord,
    route: &str,
    health: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    progress(quiet, format!("verifying https://{route}{health}"));
    let attempts = route_verification_attempts(server, route, health);
    let mut failures = Vec::new();
    for attempt in &attempts {
        let status = TokioCommand::new("curl")
            .args(&attempt.args)
            .status()
            .await
            .map_err(|source| {
                GumgumError::structured(
                    Subsystem::Api,
                    ErrorCode::Io,
                    "failed to verify deployed route",
                )
                .likely_cause(source.to_string())
                .build()
            })?;
        if status.success() {
            return Ok(());
        }
        failures.push(format!("{} exited with {status}", attempt.display));
    }
    Err(GumgumError::structured(
        Subsystem::Api,
        ErrorCode::Io,
        "deployed route did not respond",
    )
    .likely_cause(failures.join("; "))
    .next_command(attempts[0].display.clone())
    .next_command(attempts[1].display.clone())
    .build())
}

struct RouteVerificationAttempt {
    args: Vec<String>,
    display: String,
}

fn route_verification_attempts(
    server: &ServerRecord,
    route: &str,
    health: &str,
) -> [RouteVerificationAttempt; 2] {
    let public = format!("https://{route}{health}");
    let host = format!("http://{}{health}", server.host);
    [
        RouteVerificationAttempt {
            args: vec![
                "-fsS".to_owned(),
                "-o".to_owned(),
                "/dev/null".to_owned(),
                public.clone(),
            ],
            display: format!("curl -fsS -o /dev/null {public}"),
        },
        RouteVerificationAttempt {
            args: vec![
                "-fsS".to_owned(),
                "-o".to_owned(),
                "/dev/null".to_owned(),
                "-H".to_owned(),
                format!("Host: {route}"),
                host.clone(),
            ],
            display: format!("curl -fsS -o /dev/null -H 'Host: {route}' {host}"),
        },
    ]
}

#[cfg(test)]
mod deploy_hardening_tests {
    use super::*;
    use gumgum_core::{Worker, WorkerManifest};

    #[test]
    fn local_registry_image_uses_tunnel_loopback_for_push() {
        assert_eq!(
            local_push_image("127.0.0.1:55000/dev.leostera/root/api:gg1", 55001),
            "localhost:55001/dev.leostera/root/api:gg1"
        );
    }

    #[test]
    fn env_prefixed_container_names_keep_gumgum_namespace_first() {
        assert_eq!(
            env_prefixed_container_name("gumgum-dev-leostera-api", crate::DeployEnv::Preview),
            "gumgum-preview-dev-leostera-api"
        );
        assert_eq!(
            env_prefixed_container_name("gumgum-dev-leostera-api", crate::DeployEnv::Prod),
            "gumgum-prod-dev-leostera-api"
        );
    }

    fn temp_test_path(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("gumgum-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn grafana_artifact_loader_reports_missing_file() {
        let path = temp_test_path("missing-grafana.json");
        let error = load_grafana_artifact_content(path.to_str().unwrap()).unwrap_err();
        assert!(
            error
                .to_report()
                .message
                .contains("could not read Grafana artifact")
        );
    }

    #[test]
    fn grafana_artifact_loader_reports_invalid_json() {
        let path = temp_test_path("invalid-grafana.json");
        std::fs::write(&path, "{ not json").unwrap();
        let error = load_grafana_artifact_content(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            error.to_report().code,
            ErrorCode::ManifestParseFailed
        ));
    }

    #[test]
    fn grafana_artifact_loader_accepts_valid_json() {
        let path = temp_test_path("valid-grafana.json");
        std::fs::write(&path, r#"{"title":"API Overview"}"#).unwrap();
        let value = load_grafana_artifact_content(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(value["title"], "API Overview");
    }

    #[test]
    fn dry_run_deploy_report_includes_grafana_artifacts() {
        let manifest = WorkerManifest {
            project: Some(gumgum_core::Project {
                namespace: "kava-fund".to_owned(),
            }),
            worker: Worker {
                name: "api".to_owned(),
                image: None,
                build_context: None,
                command: None,
                port: None,
                checks: Default::default(),
                health: None,
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: Vec::new(),
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Default::default(),
            secrets: Vec::new(),
            observability: Some(gumgum_core::Observability {
                enable: true,
                prometheus_metrics: "/_/metrics".to_owned(),
                grafana: Some(gumgum_core::GrafanaObservability {
                    sources: Some("../grafana/sources.json".to_owned()),
                }),
            }),
            dashboards: vec![gumgum_core::Dashboard {
                name: "API Overview".to_owned(),
                path: "../grafana/dashboards/api-overview.json".to_owned(),
            }],
            limits: None,
        };

        let report = deploy_report(
            std::path::PathBuf::from("examples/visit-counter/api/gumgum.toml"),
            &manifest,
            None,
            Some("kava.fund"),
            None,
            true,
            crate::DeployEnv::Preview,
        );

        assert!(report.dry_run);
        assert_eq!(report.grafana.len(), 2);
        assert!(
            report
                .plan
                .iter()
                .any(|line| line.contains("provision Grafana dashboard kava-fund / API Overview"))
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("API Overview"));
        assert!(json.contains("datasource"));
    }

    #[test]
    fn grafana_artifact_plan_names_project_scoped_files() {
        let manifest = WorkerManifest {
            project: Some(gumgum_core::Project {
                namespace: "kava-fund".to_owned(),
            }),
            worker: Worker {
                name: "api".to_owned(),
                image: None,
                build_context: None,
                command: None,
                port: None,
                checks: Default::default(),
                health: None,
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: Vec::new(),
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Default::default(),
            secrets: Vec::new(),
            observability: Some(gumgum_core::Observability {
                enable: true,
                prometheus_metrics: "/_/metrics".to_owned(),
                grafana: Some(gumgum_core::GrafanaObservability {
                    sources: Some("../grafana/sources.json".to_owned()),
                }),
            }),
            dashboards: vec![gumgum_core::Dashboard {
                name: "API Overview".to_owned(),
                path: "../grafana/dashboards/api-overview.json".to_owned(),
            }],
            limits: None,
        };

        let plan = grafana_artifact_plan(
            std::path::Path::new("examples/visit-counter/api/gumgum.toml"),
            &manifest,
            None,
            Some("kava.fund"),
        );

        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].kind, "datasource");
        assert_eq!(plan[0].name, "kava-fund / datasources");
        assert_eq!(plan[1].folder_path, vec!["kava.fund", "kava-fund"]);
        assert!(
            plan[1]
                .path
                .ends_with("grafana/dashboards/api-overview.json")
        );
    }

    #[test]
    fn observability_metrics_path_only_when_enabled() {
        let mut manifest = WorkerManifest {
            project: None,
            worker: Worker {
                name: "api".to_owned(),
                image: None,
                build_context: None,
                command: None,
                port: None,
                checks: Default::default(),
                health: None,
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: Vec::new(),
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Default::default(),
            secrets: Vec::new(),
            observability: None,
            dashboards: Vec::new(),
            limits: None,
        };
        assert_eq!(observability_metrics_path(&manifest), None);

        manifest.observability = Some(gumgum_core::Observability {
            enable: true,
            prometheus_metrics: "/custom/metrics".to_owned(),
            grafana: None,
        });
        assert_eq!(
            observability_metrics_path(&manifest),
            Some("/custom/metrics")
        );
    }

    #[test]
    fn route_verification_prefers_public_https_with_host_fallback() {
        let server = ServerRecord {
            name: "starbase2".to_owned(),
            host: "192.168.0.3".to_owned(),
            root_domain: "leostera.dev".to_owned(),
            test_domain: String::new(),
            health_url: "http://192.168.0.3:7777/healthz".to_owned(),
        };
        let attempts =
            route_verification_attempts(&server, "visit-counter.leostera.dev", "/_/ready");

        assert_eq!(
            attempts[0].display,
            "curl -fsS -o /dev/null https://visit-counter.leostera.dev/_/ready"
        );
        assert_eq!(
            attempts[1].display,
            "curl -fsS -o /dev/null -H 'Host: visit-counter.leostera.dev' http://192.168.0.3/_/ready"
        );
    }

    #[test]
    fn deployment_key_includes_env_without_changing_display_worker() {
        assert_eq!(
            deployment_key("api", crate::DeployEnv::Preview),
            "api@preview"
        );
        assert_eq!(deployment_key("api", crate::DeployEnv::Prod), "api@prod");
    }

    #[test]
    fn deploy_command_failure_preserves_cause_with_actionable_next_command() {
        let source =
            GumgumError::structured(Subsystem::Setup, ErrorCode::Io, "setup command failed")
                .likely_cause("Cannot connect to the Docker daemon")
                .next_command("gumgum setup <host> --domain <domain> --dry-run")
                .build();

        let report =
            deploy_command_failure("Docker image build failed", source, "docker info").to_report();

        assert_eq!(report.message, "Docker image build failed");
        assert_eq!(
            report.likely_cause.as_deref(),
            Some("Cannot connect to the Docker daemon")
        );
        assert_eq!(report.next_commands, vec!["docker info".to_owned()]);
    }

    #[test]
    fn deploy_event_stream_requires_advertised_capability() {
        let mut version = gumgum_api::DaemonVersionReport {
            ok: true,
            version: "test".to_owned(),
            git_sha: "test".to_owned(),
            target: "test".to_owned(),
            capabilities: vec!["gumgum:events".to_owned()],
        };
        assert!(!supports_deploy_event_stream(&version));

        version
            .capabilities
            .push("gumgum:deployments:stream".to_owned());
        assert!(supports_deploy_event_stream(&version));
    }

    #[test]
    fn delete_deploy_output_uses_report_typed_events() {
        let event = GumgumEvent::DeploymentFailed {
            worker: "api".to_owned(),
            environment: Some("preview".to_owned()),
            error: "deployment was not present".to_owned(),
        };
        let output = DeployOutput::Delete(DeployApplyReport {
            ok: false,
            worker: "api@preview".to_owned(),
            materialized: false,
            changed: false,
            actions: vec!["deployment was not present".to_owned()],
            reconciliation_steps: Vec::new(),
            typed_events: vec![event.clone()],
            message: "deployment was not present".to_owned(),
        });

        assert_eq!(deploy_output_events(&output), vec![event]);
    }

    #[test]
    fn deploy_route_prefers_manifest_route() {
        let report = DeployReport {
            ok: true,
            dry_run: false,
            path: "api/gumgum.toml".to_owned(),
            worker: "api".to_owned(),
            host: Some("starbase2".to_owned()),
            build_context: Some("api".to_owned()),
            image: "127.0.0.1:55000/dev.leostera/root/api:gg1".to_owned(),
            container: "gumgum-api".to_owned(),
            port: 3000,
            routes: vec!["api.visit-counter.leostera.test".to_owned()],
            health_url: None,
            grafana: Vec::new(),
            plan: Vec::new(),
            plan_graph: PlanGraph::default(),
            events: Vec::new(),
            message: String::new(),
        };
        let server = ServerRecord {
            name: "starbase2".to_owned(),
            host: "starbase2".to_owned(),
            root_domain: "leostera.dev".to_owned(),
            test_domain: "leostera.test".to_owned(),
            health_url: "http://starbase2:4747/health".to_owned(),
        };

        assert_eq!(
            deploy_route(&report, &server),
            Some("api.visit-counter.leostera.test".to_owned())
        );
    }
}

pub(crate) fn print_deploy_output(json: bool, output: &DeployOutput) {
    if json {
        let events = deploy_output_events(output);
        if events.is_empty() {
            crate::print_value(true, output);
        } else {
            print_events_json_lines(events);
        }
    } else {
        crate::presentation::Presenter::new().deploy_output(output);
    }
}

fn deploy_output_events(output: &DeployOutput) -> Vec<GumgumEvent> {
    match output {
        DeployOutput::Worker(report) => report.events.clone(),
        DeployOutput::Workspace(report) => report
            .workers
            .iter()
            .flat_map(|worker| worker.events.clone())
            .collect(),
        DeployOutput::Delete(report) => report.typed_events.clone(),
    }
}
