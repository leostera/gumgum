#![allow(clippy::items_after_test_module)]

use crate::{DeployArgs, progress, resolve_server};
use gumgum_api::{DeployApplyReport, DeployRequest, DeploymentDeleteRequest, ServerRecord};
use gumgum_core::{
    ConfigStore, DeploymentDescriptor, ErrorCode, GumgumError, ManifestKind, PlanGraph, Subsystem,
    WorkerManifest, load_worker_path, load_workspace_path,
    run_setup_command_streaming as run_command_streaming, validate_path,
};
use serde::Serialize;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command as TokioCommand;

use crate::{deploy_executor::DeployExecutor, server_client::ServerClient};

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
    pub(crate) plan: Vec<String>,
    pub(crate) plan_graph: PlanGraph,
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
                        worker: manifest.worker.name,
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
                args.prod,
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
                    args.prod,
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
    prod: bool,
    quiet: bool,
) -> gumgum_core::Result<DeployReport> {
    let mut report = deploy_report(
        path,
        manifest,
        project.name,
        project.domain,
        server.as_ref(),
        dry_run,
        prod,
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
        .next_command("gumgum setup <host> --root-domain <domain>")
        .build()
    })?;
    DeployExecutor::new(&server, quiet)
        .ensure_manifest_bindings(manifest, project.name)
        .await?;
    run_remote_deploy(&server, manifest, &report, quiet).await?;
    report.ok = true;
    report.dry_run = false;
    report.message = match &report.health_url {
        Some(health_url) => format!(
            "deployed {} to {}; health verified at {}",
            report.worker, server.host, health_url
        ),
        None => format!("deployed {} to {}", report.worker, server.host),
    };
    Ok(report)
}

fn deploy_report(
    path: PathBuf,
    manifest: &WorkerManifest,
    project_name: Option<&str>,
    project_domain: Option<&str>,
    server: Option<&ServerRecord>,
    dry_run: bool,
    prod: bool,
) -> DeployReport {
    let descriptor = DeploymentDescriptor::from_manifest_in_project(
        &path,
        manifest,
        project_name,
        project_domain,
        server,
        prod,
    );
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
        plan: descriptor.plan,
        plan_graph: descriptor.plan_graph,
        message: if dry_run {
            format!(
                "validated worker manifest for {} deploy; no containers changed",
                if prod { "prod" } else { "test" }
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
    build_result?;
    if let Some(push_result) = push_result {
        push_result?;
    }

    progress(
        quiet,
        format!("asking gumgumd on {host} to reconcile {}", report.worker),
    );
    let request = DeployRequest {
        worker: report.worker.clone(),
        image: report.image.clone(),
        container: report.container.clone(),
        route: route.clone(),
        port: report.port,
        health: manifest.worker.ready_check_path().to_owned(),
    };
    apply_deploy_via_daemon(host, &request).await?;
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
        verify_route(
            server,
            route.as_ref().expect("ingress deploy has a route"),
            manifest.worker.ready_check_path(),
            quiet,
        )
        .await
    }
}

async fn apply_deploy_via_daemon(
    host: &str,
    request: &DeployRequest,
) -> gumgum_core::Result<DeployApplyReport> {
    let report = ServerClient::new(host).deploy(request).await?;
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

async fn wait_for_remote_registry(host: &str, quiet: bool) -> gumgum_core::Result<()> {
    progress(
        quiet,
        format!("checking GumGum.dev registry managed by daemon on {host}"),
    );
    let script = "for i in $(seq 1 20); do if docker inspect -f '{{.State.Running}}' gumgum-registry 2>/dev/null | grep -q true; then exit 0; fi; sleep 0.5; done; echo 'gumgum-registry is not running; is gumgumd active?' >&2; exit 1";
    run_command_streaming(TokioCommand::new("ssh").arg(host).arg(script), quiet).await
}

fn local_push_image(image: &str, tunnel_port: u16) -> String {
    image.replacen("127.0.0.1:55000", &format!("localhost:{tunnel_port}"), 1)
}

fn deploy_route(report: &DeployReport, _server: &ServerRecord) -> Option<String> {
    report.routes.first().cloned()
}

async fn verify_route(
    server: &ServerRecord,
    route: &str,
    health: &str,
    quiet: bool,
) -> gumgum_core::Result<()> {
    progress(quiet, format!("verifying https://{route}{health}"));
    let url = format!("http://{}{health}", server.host);
    let status = TokioCommand::new("curl")
        .arg("-fsS")
        .arg("-H")
        .arg(format!("Host: {route}"))
        .arg(url)
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
        Ok(())
    } else {
        Err(GumgumError::structured(
            Subsystem::Api,
            ErrorCode::Io,
            "deployed route did not respond",
        )
        .likely_cause(format!("curl exited with {status}"))
        .next_command(format!(
            "curl -H 'Host: {route}' http://{}{health}",
            server.host
        ))
        .build())
    }
}

#[cfg(test)]
mod deploy_hardening_tests {
    use super::*;

    #[test]
    fn local_registry_image_uses_tunnel_loopback_for_push() {
        assert_eq!(
            local_push_image("127.0.0.1:55000/dev.leostera/root/api:gg1", 55001),
            "localhost:55001/dev.leostera/root/api:gg1"
        );
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
            plan: Vec::new(),
            plan_graph: PlanGraph::default(),
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
        crate::print_value(true, output);
    } else {
        crate::presentation::Presenter::new().deploy_output(output);
    }
}
