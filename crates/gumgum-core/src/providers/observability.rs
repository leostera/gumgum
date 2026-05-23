use crate::{Capability, ContainerRunSpec, DockerEngine};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::types::{ProviderSpec, ProviderStatus};

const GUMGUM_NETWORK: &str = "gumgum-network";

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Observability,
        provider: "observability.platform".to_owned(),
        container: "gumgum-otel".to_owned(),
        image: "otel/opentelemetry-collector-contrib:latest".to_owned(),
        port: 4317,
        protocol: "otlp".to_owned(),
    }
}

pub(crate) fn actions(_safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure observability.platform provider is running".to_owned(),
        format!("publish DNS {dns} to observability.platform"),
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<String> {
    vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")]
}

pub(crate) async fn ensure_platform_stack(root_domain: &str) -> crate::Result<Vec<String>> {
    let mut actions = Vec::new();
    for provider in platform_specs(root_domain) {
        actions.extend(ensure_platform_container(&provider, root_domain).await?);
    }
    Ok(actions)
}

async fn ensure_platform_container(
    provider: &ProviderSpec,
    root_domain: &str,
) -> crate::Result<Vec<String>> {
    let docker = DockerEngine::local()?;
    docker.ensure_network(GUMGUM_NETWORK).await?;
    let desired = platform_run_spec(provider, root_domain);
    if let Some(existing) = docker.inspect_container(&provider.container).await? {
        let desired_fingerprint = platform_fingerprint(&desired);
        if existing.labels.get("gumgum.platform.fingerprint") == Some(&desired_fingerprint) {
            docker.start_container(&provider.container).await?;
            return Ok(vec![format!("started existing {}", provider.container)]);
        }
        docker.remove_container_force(&provider.container).await?;
    }
    docker.pull_image(&provider.image).await?;
    docker.create_and_start_container(desired).await?;
    Ok(vec![format!(
        "created platform service {} ({})",
        provider.container, provider.provider
    )])
}

fn platform_run_spec(provider: &ProviderSpec, root_domain: &str) -> ContainerRunSpec {
    let env = platform_env(provider, root_domain);
    let command = platform_command(provider);
    let binds = platform_binds(provider);
    let mut labels = platform_labels(provider);
    labels.insert(
        "gumgum.platform.fingerprint".to_owned(),
        platform_fingerprint_parts(&env, &command, &binds),
    );
    ContainerRunSpec {
        name: provider.container.clone(),
        image: provider.image.clone(),
        network: GUMGUM_NETWORK.to_owned(),
        restart_unless_stopped: true,
        labels,
        env,
        binds,
        ports: Vec::new(),
        command,
        entrypoint: Vec::new(),
    }
}

fn platform_labels(provider: &ProviderSpec) -> HashMap<String, String> {
    HashMap::from([
        ("gumgum.managed".to_owned(), "platform".to_owned()),
        (
            "gumgum.platform.service".to_owned(),
            provider.container.trim_start_matches("gumgum-").to_owned(),
        ),
        ("gumgum.capability".to_owned(), "observability".to_owned()),
    ])
}

fn platform_env(provider: &ProviderSpec, root_domain: &str) -> Vec<(String, String)> {
    if provider.container == "gumgum-grafana" {
        vec![
            ("GF_USERS_ALLOW_SIGN_UP".to_owned(), "false".to_owned()),
            ("GF_SECURITY_ADMIN_USER".to_owned(), "gumgum".to_owned()),
            (
                "GF_SECURITY_ADMIN_PASSWORD".to_owned(),
                std::env::var("GUMGUM_GRAFANA_ADMIN_PASSWORD")
                    .unwrap_or_else(|_| "gumgum-local-dev".to_owned()),
            ),
            (
                "GF_SERVER_ROOT_URL".to_owned(),
                format!("https://grafana.{root_domain}/"),
            ),
        ]
    } else {
        Vec::new()
    }
}

fn platform_binds(provider: &ProviderSpec) -> Vec<String> {
    match provider.container.as_str() {
        "gumgum-grafana" => vec!["gumgum-grafana-data:/var/lib/grafana".to_owned()],
        "gumgum-prometheus" => vec!["gumgum-prometheus-data:/prometheus".to_owned()],
        "gumgum-loki" => vec!["gumgum-loki-data:/loki".to_owned()],
        _ => Vec::new(),
    }
}

fn platform_fingerprint(spec: &ContainerRunSpec) -> String {
    platform_fingerprint_parts(&spec.env, &spec.command, &spec.binds)
}

fn platform_fingerprint_parts(
    env: &[(String, String)],
    command: &[String],
    binds: &[String],
) -> String {
    let mut parts = env
        .iter()
        .map(|(name, value)| format!("env:{name}={value}"))
        .chain(command.iter().map(|value| format!("cmd:{value}")))
        .chain(binds.iter().map(|value| format!("bind:{value}")))
        .collect::<Vec<_>>();
    parts.sort();
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("v2:{:016x}", hasher.finish())
}

fn platform_command(provider: &ProviderSpec) -> Vec<String> {
    match provider.container.as_str() {
        "gumgum-loki" => vec!["-config.file=/etc/loki/local-config.yaml".to_owned()],
        "gumgum-tempo" => vec![
            "-target=all".to_owned(),
            "-server.http-listen-port=3200".to_owned(),
            "-storage.trace.backend=local".to_owned(),
            "-storage.trace.local.path=/tmp/tempo/traces".to_owned(),
            "-auth.enabled=false".to_owned(),
        ],
        _ => Vec::new(),
    }
}

pub async fn apply_grafana_artifact(
    kind: &str,
    name: &str,
    content: serde_json::Value,
) -> crate::Result<Vec<String>> {
    let password = std::env::var("GUMGUM_GRAFANA_ADMIN_PASSWORD")
        .unwrap_or_else(|_| "gumgum-local-dev".to_owned());
    let client = reqwest::Client::new();
    let grafana = DockerEngine::local()?
        .inspect_container("gumgum-grafana")
        .await?
        .ok_or_else(|| {
            crate::GumgumError::structured(
                crate::Subsystem::Setup,
                crate::ErrorCode::Io,
                "Grafana platform container is not running",
            )
            .next_command("gumgum server add <host> --root-domain <domain>")
            .build()
        })?;
    let ip = grafana
        .networks
        .get(GUMGUM_NETWORK)
        .filter(|ip| !ip.is_empty())
        .ok_or_else(|| {
            crate::GumgumError::structured(
                crate::Subsystem::Setup,
                crate::ErrorCode::Io,
                "Grafana platform container is not attached to gumgum network",
            )
            .build()
        })?;
    let base = format!("http://{ip}:3000");
    match kind {
        "datasource" => {
            let datasources = content
                .get("datasources")
                .and_then(|value| value.as_array())
                .ok_or_else(|| {
                    crate::GumgumError::structured(
                        crate::Subsystem::Setup,
                        crate::ErrorCode::InvalidArgs,
                        "Grafana datasource artifact must contain a datasources array",
                    )
                    .build()
                })?;
            let mut actions = Vec::new();
            for datasource in datasources {
                let datasource_name = datasource
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unnamed");
                let response = client
                    .post(format!("{base}/api/datasources"))
                    .basic_auth("gumgum", Some(&password))
                    .json(datasource)
                    .send()
                    .await
                    .map_err(grafana_error)?;
                if response.status().is_success() {
                    actions.push(format!("created Grafana datasource {datasource_name}"));
                } else if response.status().as_u16() == 409 {
                    let uid =
                        grafana_datasource_uid(&client, &base, &password, datasource_name).await?;
                    let update = client
                        .put(format!("{base}/api/datasources/uid/{uid}"))
                        .basic_auth("gumgum", Some(&password))
                        .json(datasource)
                        .send()
                        .await
                        .map_err(grafana_error)?;
                    if update.status().is_success() {
                        actions.push(format!("updated Grafana datasource {datasource_name}"));
                    } else {
                        return Err(grafana_response_error(update).await);
                    }
                } else {
                    return Err(grafana_response_error(response).await);
                }
            }
            Ok(actions)
        }
        "dashboard" => {
            let response = client
                .post(format!("{base}/api/dashboards/db"))
                .basic_auth("gumgum", Some(&password))
                .json(&serde_json::json!({
                    "dashboard": content,
                    "overwrite": true,
                    "message": format!("gumgum apply {name}"),
                }))
                .send()
                .await
                .map_err(grafana_error)?;
            if response.status().is_success() {
                Ok(vec![format!("applied Grafana dashboard {name}")])
            } else {
                Err(grafana_response_error(response).await)
            }
        }
        other => Err(crate::GumgumError::structured(
            crate::Subsystem::Setup,
            crate::ErrorCode::InvalidArgs,
            format!("unsupported Grafana artifact kind {other}"),
        )
        .build()),
    }
}

async fn grafana_datasource_uid(
    client: &reqwest::Client,
    base: &str,
    password: &str,
    name: &str,
) -> crate::Result<String> {
    let response = client
        .get(format!(
            "{base}/api/datasources/name/{}",
            grafana_path_escape(name)
        ))
        .basic_auth("gumgum", Some(password))
        .send()
        .await
        .map_err(grafana_error)?;
    if !response.status().is_success() {
        return Err(grafana_response_error(response).await);
    }
    let value: serde_json::Value = response.json().await.map_err(grafana_error)?;
    value
        .get("uid")
        .and_then(|uid| uid.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            crate::GumgumError::structured(
                crate::Subsystem::Setup,
                crate::ErrorCode::Io,
                format!("Grafana datasource {name} did not include a uid"),
            )
            .build()
        })
}

fn grafana_path_escape(value: &str) -> String {
    value.replace(' ', "%20").replace('/', "%2F")
}

fn grafana_error(error: reqwest::Error) -> crate::GumgumError {
    crate::GumgumError::structured(
        crate::Subsystem::Setup,
        crate::ErrorCode::Io,
        "could not reach Grafana API",
    )
    .likely_cause(error.to_string())
    .build()
}

async fn grafana_response_error(response: reqwest::Response) -> crate::GumgumError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    crate::GumgumError::structured(
        crate::Subsystem::Setup,
        crate::ErrorCode::Io,
        format!("Grafana API returned {status}"),
    )
    .likely_cause(body)
    .build()
}

pub(crate) async fn platform_statuses() -> Vec<ProviderStatus> {
    let mut statuses = Vec::new();
    for provider in platform_specs("example.invalid") {
        let running = super::docker::running(&provider.container).await;
        statuses.push(ProviderStatus {
            capability: Capability::Observability,
            provider: provider.provider,
            container: provider.container,
            image: provider.image,
            port: provider.port,
            running,
        });
    }
    statuses
}

fn platform_specs(_root_domain: &str) -> Vec<ProviderSpec> {
    vec![
        spec(),
        ProviderSpec {
            capability: Capability::Observability,
            provider: "prometheus.platform".to_owned(),
            container: "gumgum-prometheus".to_owned(),
            image: "prom/prometheus:latest".to_owned(),
            port: 9090,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "grafana.platform".to_owned(),
            container: "gumgum-grafana".to_owned(),
            image: "grafana/grafana:latest".to_owned(),
            port: 3000,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "loki.platform".to_owned(),
            container: "gumgum-loki".to_owned(),
            image: "grafana/loki:latest".to_owned(),
            port: 3100,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "tempo.platform".to_owned(),
            container: "gumgum-tempo".to_owned(),
            image: "grafana/tempo:latest".to_owned(),
            port: 3200,
            protocol: "http".to_owned(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observability_platform_specs_use_singleton_container_names() {
        let specs = platform_specs("leostera.dev");
        assert!(
            specs
                .iter()
                .any(|spec| spec.container == "gumgum-prometheus")
        );
        assert!(specs.iter().any(|spec| spec.container == "gumgum-grafana"));
        assert!(specs.iter().any(|spec| spec.container == "gumgum-loki"));
        assert!(specs.iter().any(|spec| spec.container == "gumgum-tempo"));
        assert!(specs.iter().all(|spec| !spec.container.contains("preview")));
        assert!(specs.iter().all(|spec| !spec.container.contains("prod")));
    }

    #[test]
    fn grafana_platform_env_disables_signup_and_sets_admin() {
        let grafana = platform_specs("leostera.dev")
            .into_iter()
            .find(|spec| spec.container == "gumgum-grafana")
            .unwrap();
        let env = platform_env(&grafana, "leostera.dev");
        assert!(env.contains(&("GF_USERS_ALLOW_SIGN_UP".to_owned(), "false".to_owned())));
        assert!(
            env.iter()
                .any(|(name, _)| name == "GF_SECURITY_ADMIN_PASSWORD")
        );
        assert!(env.contains(&(
            "GF_SERVER_ROOT_URL".to_owned(),
            "https://grafana.leostera.dev/".to_owned()
        )));
    }

    #[test]
    fn platform_labels_mark_observability_singletons() {
        let labels = platform_labels(&spec());
        assert_eq!(
            labels.get("gumgum.managed").map(String::as_str),
            Some("platform")
        );
        assert_eq!(
            labels.get("gumgum.capability").map(String::as_str),
            Some("observability")
        );
    }

    #[test]
    fn grafana_path_escape_handles_datasource_names() {
        assert_eq!(
            grafana_path_escape("Project / Prometheus"),
            "Project%20%2F%20Prometheus"
        );
    }
}
