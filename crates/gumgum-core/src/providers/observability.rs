use crate::{Capability, ContainerRunSpec, CoreAction, CoreActions, DockerEngine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use super::types::{ProviderSpec, ProviderStatus};

const GUMGUM_NETWORK: &str = "gumgum-network";
const PLATFORM_FINGERPRINT_VERSION: &str = "v7";

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

pub(crate) fn actions(_safe_name: &str, dns: &str) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Observability,
            provider: "observability.platform".to_owned(),
        },
        CoreAction::DnsPublished {
            dns: dns.to_owned(),
            provider: "observability.platform".to_owned(),
        },
    ]
}

pub(crate) fn connection_examples(_name: &str, dns: &str) -> Vec<crate::ConnectionExample> {
    vec![crate::ConnectionExample::OtelEndpoint {
        dns: dns.to_owned(),
    }]
}

pub(crate) async fn ensure_platform_stack(root_domain: &str) -> crate::Result<CoreActions> {
    let mut actions = Vec::new();
    for provider in platform_specs(root_domain) {
        actions.extend(ensure_platform_container(&provider, root_domain).await?);
    }
    Ok(actions)
}

async fn ensure_platform_container(
    provider: &ProviderSpec,
    root_domain: &str,
) -> crate::Result<CoreActions> {
    let docker = DockerEngine::local()?;
    docker.ensure_network(GUMGUM_NETWORK).await?;
    if provider.container == "gumgum-prometheus" {
        let targets = load_prometheus_scrapes()?;
        ensure_prometheus_config(&targets)?;
    }
    if provider.container == "gumgum-otel" {
        ensure_otel_config()?;
    }
    if provider.container == "gumgum-tempo" {
        ensure_tempo_config()?;
    }
    if provider.container == "gumgum-alloy" {
        ensure_alloy_config()?;
    }
    let desired = platform_run_spec(provider, root_domain);
    if let Some(existing) = docker.inspect_container(&provider.container).await? {
        let desired_fingerprint = platform_fingerprint(&desired);
        if existing.labels.get("gumgum.platform.fingerprint") == Some(&desired_fingerprint) {
            docker.start_container(&provider.container).await?;
            return Ok(vec![CoreAction::PlatformServiceStarted {
                container: provider.container.clone(),
            }]);
        }
        docker.remove_container_force(&provider.container).await?;
    }
    docker.pull_image(&provider.image).await?;
    docker.create_and_start_container(desired).await?;
    Ok(vec![CoreAction::PlatformServiceCreated {
        provider: provider.provider.clone(),
        container: provider.container.clone(),
    }])
}

fn platform_run_spec(provider: &ProviderSpec, root_domain: &str) -> ContainerRunSpec {
    let env = platform_env(provider, root_domain);
    let command = platform_command(provider);
    let binds = platform_binds(provider);
    let mut labels = platform_labels(provider, root_domain);
    labels.insert(
        "gumgum.platform.fingerprint".to_owned(),
        platform_fingerprint_parts(&provider.image, &env, &command, &binds),
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

fn platform_labels(provider: &ProviderSpec, root_domain: &str) -> HashMap<String, String> {
    let mut labels = HashMap::from([
        ("gumgum.managed".to_owned(), "platform".to_owned()),
        (
            "gumgum.platform.service".to_owned(),
            provider.container.trim_start_matches("gumgum-").to_owned(),
        ),
        ("gumgum.capability".to_owned(), "observability".to_owned()),
    ]);
    if provider.container == "gumgum-grafana" {
        labels.insert("caddy".to_owned(), format!("grafana.{root_domain}"));
        labels.insert(
            "caddy.reverse_proxy".to_owned(),
            "{{upstreams 3000}}".to_owned(),
        );
        labels.insert("caddy.tls".to_owned(), "internal".to_owned());
    }
    labels
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
    } else if provider.container == "gumgum-docker-proxy" {
        vec![
            ("CONTAINERS".to_owned(), "1".to_owned()),
            ("NETWORKS".to_owned(), "1".to_owned()),
            ("INFO".to_owned(), "1".to_owned()),
        ]
    } else {
        Vec::new()
    }
}

fn platform_binds(provider: &ProviderSpec) -> Vec<String> {
    match provider.container.as_str() {
        "gumgum-grafana" => vec!["/gumgum/volumes/platform/grafana:/var/lib/grafana".to_owned()],
        "gumgum-prometheus" => vec![
            "/gumgum/volumes/platform/prometheus:/prometheus".to_owned(),
            format!(
                "{}:/etc/prometheus/prometheus.yml:ro",
                prometheus_config_path().display()
            ),
        ],
        "gumgum-otel" => vec![format!(
            "{}:/etc/otelcol-contrib/config.yaml:ro",
            otel_config_path().display()
        )],
        "gumgum-alloy" => vec![
            "/var/run/docker.sock:/var/run/docker.sock:ro".to_owned(),
            "/gumgum/volumes/platform/alloy:/var/lib/alloy".to_owned(),
            format!(
                "{}:/etc/alloy/config.alloy:ro",
                alloy_config_path().display()
            ),
        ],
        "gumgum-docker-proxy" => vec!["/var/run/docker.sock:/var/run/docker.sock:ro".to_owned()],
        "gumgum-node-exporter" => vec!["/:/host:ro,rslave".to_owned()],
        "gumgum-cadvisor" => vec![
            "/:/rootfs:ro".to_owned(),
            "/var/run:/var/run:ro".to_owned(),
            "/sys:/sys:ro".to_owned(),
            "/var/lib/docker:/var/lib/docker:ro".to_owned(),
            "/dev/disk:/dev/disk:ro".to_owned(),
        ],
        "gumgum-loki" => vec!["/gumgum/volumes/platform/loki:/loki".to_owned()],
        "gumgum-tempo" => vec![
            "/gumgum/volumes/platform/tempo:/tmp/tempo".to_owned(),
            format!("{}:/etc/tempo.yaml:ro", tempo_config_path().display()),
        ],
        _ => Vec::new(),
    }
}

fn platform_fingerprint(spec: &ContainerRunSpec) -> String {
    platform_fingerprint_parts(&spec.image, &spec.env, &spec.command, &spec.binds)
}

fn platform_fingerprint_parts(
    image: &str,
    env: &[(String, String)],
    command: &[String],
    binds: &[String],
) -> String {
    let mut parts = std::iter::once(format!("version:{PLATFORM_FINGERPRINT_VERSION}"))
        .chain(std::iter::once(format!("image:{image}")))
        .chain(
            env.iter()
                .map(|(name, value)| format!("env:{name}={value}"))
                .chain(command.iter().map(|value| format!("cmd:{value}")))
                .chain(binds.iter().map(|value| format!("bind:{value}"))),
        )
        .collect::<Vec<_>>();
    parts.sort();
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{PLATFORM_FINGERPRINT_VERSION}:{:016x}", hasher.finish())
}

fn platform_command(provider: &ProviderSpec) -> Vec<String> {
    match provider.container.as_str() {
        "gumgum-prometheus" => vec![
            "--config.file=/etc/prometheus/prometheus.yml".to_owned(),
            "--storage.tsdb.path=/prometheus".to_owned(),
            "--web.enable-lifecycle".to_owned(),
        ],
        "gumgum-loki" => vec!["-config.file=/etc/loki/local-config.yaml".to_owned()],
        "gumgum-tempo" => vec![
            "-config.file=/etc/tempo.yaml".to_owned(),
            "-target=all".to_owned(),
        ],
        "gumgum-alloy" => vec![
            "run".to_owned(),
            "--storage.path=/var/lib/alloy".to_owned(),
            "--server.http.listen-addr=0.0.0.0:12345".to_owned(),
            "/etc/alloy/config.alloy".to_owned(),
        ],
        "gumgum-node-exporter" => vec!["--path.rootfs=/host".to_owned()],
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PrometheusScrapeTarget {
    worker: String,
    environment: String,
    container: String,
    port: u16,
    metrics_path: String,
}

pub async fn configure_prometheus_scrape(
    worker: &str,
    environment: &str,
    container: &str,
    port: u16,
    metrics_path: &str,
) -> crate::Result<CoreActions> {
    let mut targets = load_prometheus_scrapes()?;
    let target = PrometheusScrapeTarget {
        worker: worker.to_owned(),
        environment: environment.to_owned(),
        container: container.to_owned(),
        port,
        metrics_path: metrics_path.to_owned(),
    };
    targets.retain(|existing| {
        !(existing.worker == target.worker && existing.environment == target.environment)
    });
    targets.push(target.clone());
    targets.sort_by(|left, right| {
        (&left.environment, &left.worker).cmp(&(&right.environment, &right.worker))
    });
    save_prometheus_scrapes(&targets)?;
    ensure_prometheus_config(&targets)?;
    reload_prometheus().await?;
    Ok(vec![CoreAction::PrometheusScrapeConfigured {
        worker: target.worker,
        environment: target.environment,
        container: target.container,
        port: target.port,
        metrics_path: target.metrics_path,
    }])
}

fn prometheus_state_dir() -> PathBuf {
    std::env::var_os("GUMGUM_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".gumgum")))
        .unwrap_or_else(|| PathBuf::from(".gumgum"))
        .join("observability")
}

fn prometheus_config_path() -> PathBuf {
    prometheus_state_dir().join("prometheus.yml")
}

fn alloy_config_path() -> PathBuf {
    prometheus_state_dir().join("alloy.river")
}

fn otel_config_path() -> PathBuf {
    prometheus_state_dir().join("otel-collector.yaml")
}

fn tempo_config_path() -> PathBuf {
    prometheus_state_dir().join("tempo.yaml")
}

fn prometheus_scrapes_path() -> PathBuf {
    prometheus_state_dir().join("prometheus-scrapes.json")
}

fn load_prometheus_scrapes() -> crate::Result<Vec<PrometheusScrapeTarget>> {
    let path = prometheus_scrapes_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| io_error("could not read Prometheus scrape state", error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        crate::GumgumError::structured(
            crate::Subsystem::Setup,
            crate::ErrorCode::ManifestParseFailed,
            "could not parse Prometheus scrape state",
        )
        .likely_cause(error.to_string())
        .build()
    })
}

fn save_prometheus_scrapes(targets: &[PrometheusScrapeTarget]) -> crate::Result<()> {
    let path = prometheus_scrapes_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| io_error("could not create Prometheus state directory", error))?;
    }
    let bytes = serde_json::to_vec_pretty(targets).map_err(|error| {
        crate::GumgumError::structured(
            crate::Subsystem::Setup,
            crate::ErrorCode::Io,
            "could not serialize Prometheus scrape state",
        )
        .likely_cause(error.to_string())
        .build()
    })?;
    std::fs::write(path, bytes)
        .map_err(|error| io_error("could not write Prometheus scrape state", error))
}

fn ensure_alloy_config() -> crate::Result<()> {
    let path = alloy_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| io_error("could not create Alloy config directory", error))?;
    }
    std::fs::write(path, alloy_config())
        .map_err(|error| io_error("could not write Alloy config", error))
}

fn ensure_otel_config() -> crate::Result<()> {
    let path = otel_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| io_error("could not create OpenTelemetry config directory", error))?;
    }
    std::fs::write(path, otel_config())
        .map_err(|error| io_error("could not write OpenTelemetry config", error))
}

fn ensure_tempo_config() -> crate::Result<()> {
    let path = tempo_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| io_error("could not create Tempo config directory", error))?;
    }
    std::fs::write(path, tempo_config())
        .map_err(|error| io_error("could not write Tempo config", error))
}

fn otel_config() -> String {
    r#"receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch: {}

exporters:
  otlp/tempo:
    endpoint: gumgum-tempo:4317
    tls:
      insecure: true

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp/tempo]
"#
    .to_owned()
}

fn tempo_config() -> String {
    r#"server:
  http_listen_port: 3200
  grpc_listen_port: 9095

distributor:
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: 0.0.0.0:4317
        http:
          endpoint: 0.0.0.0:4318

storage:
  trace:
    backend: local
    local:
      path: /tmp/tempo/traces
    wal:
      path: /tmp/tempo/wal

"#
    .to_owned()
}

fn alloy_config() -> String {
    r#"discovery.docker "gumgum" {
  host             = "unix:///var/run/docker.sock"
  refresh_interval = "15s"
}

discovery.relabel "gumgum_logs" {
  targets = discovery.docker.gumgum.targets

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_managed"]
    regex         = ".+"
    action        = "keep"
  }

  rule {
    source_labels = ["__meta_docker_container_name"]
    regex         = "/(.*)"
    target_label  = "container"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_managed"]
    target_label  = "gumgum_managed"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_environment"]
    target_label  = "environment"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_project"]
    target_label  = "project"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_domain"]
    target_label  = "domain"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_worker"]
    target_label  = "worker"
  }

  rule {
    source_labels = ["__meta_docker_container_label_gumgum_platform_service"]
    target_label  = "platform_service"
  }

  rule {
    target_label = "job"
    replacement  = "gumgum-docker-logs"
  }
}

loki.source.docker "gumgum" {
  host             = "unix:///var/run/docker.sock"
  targets          = discovery.relabel.gumgum_logs.output
  relabel_rules    = discovery.relabel.gumgum_logs.rules
  refresh_interval = "15s"
  labels           = { source = "docker" }
  forward_to       = [loki.write.gumgum.receiver]
}

loki.write "gumgum" {
  endpoint {
    url = "http://gumgum-loki:3100/loki/api/v1/push"
  }
}
"#
    .to_owned()
}

fn ensure_prometheus_config(targets: &[PrometheusScrapeTarget]) -> crate::Result<()> {
    let path = prometheus_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| io_error("could not create Prometheus config directory", error))?;
    }
    let mut config = prometheus_config_base();
    for target in targets {
        config.push_str(&format!(
            "  - job_name: gumgum-{}-{}\n    metrics_path: '{}'\n    static_configs:\n      - targets: ['{}:{}']\n        labels:\n          worker: '{}'\n          environment: '{}'\n",
            yaml_scalar(&target.environment),
            yaml_scalar(&target.worker),
            yaml_scalar(&target.metrics_path),
            yaml_scalar(&target.container),
            target.port,
            yaml_scalar(&target.worker),
            yaml_scalar(&target.environment)
        ));
    }
    std::fs::write(path, config)
        .map_err(|error| io_error("could not write Prometheus config", error))
}

async fn reload_prometheus() -> crate::Result<()> {
    let docker = DockerEngine::local()?;
    if !docker.container_running("gumgum-prometheus").await? {
        return Ok(());
    }
    let prometheus = docker
        .inspect_container("gumgum-prometheus")
        .await?
        .and_then(|container| container.networks.get(GUMGUM_NETWORK).cloned());
    let Some(ip) = prometheus.filter(|ip| !ip.is_empty()) else {
        return Ok(());
    };
    let response = reqwest::Client::new()
        .post(format!("http://{ip}:9090/-/reload"))
        .send()
        .await
        .map_err(grafana_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(grafana_response_error(response).await)
    }
}

fn prometheus_config_base() -> String {
    r#"global:
  scrape_interval: 15s
  evaluation_interval: 15s
scrape_configs:
  - job_name: gumgum-platform
    static_configs:
      - targets: ['gumgum-prometheus:9090']

  - job_name: gumgum-host
    static_configs:
      - targets: ['gumgum-node-exporter:9100']

  - job_name: gumgum-containers
    static_configs:
      - targets: ['gumgum-cadvisor:8080']

  - job_name: gumgum-docker
    docker_sd_configs:
      - host: tcp://gumgum-docker-proxy:2375
        refresh_interval: 30s
    relabel_configs:
      - source_labels: [__meta_docker_container_label_prometheus_scrape]
        action: keep
        regex: "true"
      - source_labels: [__meta_docker_container_name]
        target_label: container
        regex: /(.+)
        replacement: '$1'
      - source_labels: [__meta_docker_container_label_gumgum_managed]
        target_label: gumgum_managed
      - source_labels: [__meta_docker_container_label_gumgum_environment]
        target_label: environment
      - source_labels: [__meta_docker_container_label_gumgum_worker]
        target_label: worker
      - source_labels: [__meta_docker_container_label_gumgum_platform_service]
        target_label: platform_service
      - source_labels: [__meta_docker_container_label_prometheus_path]
        action: replace
        target_label: __metrics_path__
        regex: (.+)
      - source_labels: [__meta_docker_container_label_prometheus_port, __meta_docker_network_ip]
        action: replace
        target_label: __address__
        regex: ([^;]+);(.+)
        replacement: '$2:$1'
      - source_labels: [__meta_docker_network_ip, __meta_docker_port_private]
        action: replace
        target_label: __address__
        regex: ([^;]+);(.+)
        replacement: '$1:$2'
      - action: labelmap
        regex: __meta_docker_container_label_prometheus_label_(.+)
        replacement: '$1'
"#
    .to_owned()
}

fn yaml_scalar(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "''")
}

fn io_error(message: &str, error: std::io::Error) -> crate::GumgumError {
    crate::GumgumError::structured(crate::Subsystem::Setup, crate::ErrorCode::Io, message)
        .likely_cause(error.to_string())
        .build()
}

pub async fn apply_grafana_artifact(
    kind: &str,
    name: &str,
    folder_path: &[String],
    content: serde_json::Value,
) -> crate::Result<CoreActions> {
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
                    actions.push(CoreAction::GrafanaDatasourceCreated {
                        name: datasource_name.to_owned(),
                    });
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
                        actions.push(CoreAction::GrafanaDatasourceUpdated {
                            name: datasource_name.to_owned(),
                        });
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
            let folder_uid =
                ensure_grafana_folder_path(&client, &base, &password, folder_path).await?;
            let mut dashboard = content;
            if dashboard.get("uid").and_then(|uid| uid.as_str()).is_none() {
                dashboard["uid"] =
                    serde_json::Value::String(grafana_dashboard_uid(name, folder_path));
            }
            let mut payload = serde_json::json!({
                "dashboard": dashboard,
                "overwrite": true,
                "message": format!("gumgum apply {name}"),
            });
            if let Some(folder_uid) = folder_uid {
                payload["folderUid"] = serde_json::Value::String(folder_uid);
            }
            let response = client
                .post(format!("{base}/api/dashboards/db"))
                .basic_auth("gumgum", Some(&password))
                .json(&payload)
                .send()
                .await
                .map_err(grafana_error)?;
            if response.status().is_success() {
                Ok(vec![CoreAction::GrafanaDashboardApplied {
                    name: name.to_owned(),
                }])
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

async fn ensure_grafana_folder_path(
    client: &reqwest::Client,
    base: &str,
    password: &str,
    folder_path: &[String],
) -> crate::Result<Option<String>> {
    let mut parent_uid: Option<String> = None;
    for title in folder_path.iter().filter(|title| !title.trim().is_empty()) {
        let uid = grafana_folder_uid(parent_uid.as_deref(), title);
        let response = client
            .post(format!("{base}/api/folders"))
            .basic_auth("gumgum", Some(password))
            .json(&serde_json::json!({
                "uid": uid,
                "title": title,
                "parentUid": parent_uid,
            }))
            .send()
            .await
            .map_err(grafana_error)?;
        if response.status().is_success() || matches!(response.status().as_u16(), 409 | 412) {
            parent_uid = Some(uid);
        } else {
            return Err(grafana_response_error(response).await);
        }
    }
    Ok(parent_uid)
}

fn grafana_folder_uid(parent_uid: Option<&str>, title: &str) -> String {
    let prefix = parent_uid.unwrap_or("gumgum");
    let slug = crate::sanitize_name(title);
    format!("{prefix}-{slug}").chars().take(40).collect()
}

fn grafana_dashboard_uid(name: &str, folder_path: &[String]) -> String {
    let scope = folder_path
        .iter()
        .filter(|part| !part.trim().is_empty())
        .cloned()
        .chain(std::iter::once(name.to_owned()))
        .collect::<Vec<_>>()
        .join(" /");
    format!("gumgum-{}", crate::sanitize_name(&scope))
        .chars()
        .take(40)
        .collect()
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
        ProviderSpec {
            capability: Capability::Observability,
            provider: "alloy.platform".to_owned(),
            container: "gumgum-alloy".to_owned(),
            image: "grafana/alloy:latest".to_owned(),
            port: 12345,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "node-exporter.platform".to_owned(),
            container: "gumgum-node-exporter".to_owned(),
            image: "prom/node-exporter:latest".to_owned(),
            port: 9100,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "cadvisor.platform".to_owned(),
            container: "gumgum-cadvisor".to_owned(),
            image: "gcr.io/cadvisor/cadvisor:latest".to_owned(),
            port: 8080,
            protocol: "http".to_owned(),
        },
        ProviderSpec {
            capability: Capability::Observability,
            provider: "docker-proxy.platform".to_owned(),
            container: "gumgum-docker-proxy".to_owned(),
            image: "tecnativa/docker-socket-proxy:latest".to_owned(),
            port: 2375,
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
        assert!(specs.iter().any(|spec| spec.container == "gumgum-alloy"));
        assert!(
            specs
                .iter()
                .any(|spec| spec.container == "gumgum-node-exporter")
        );
        assert!(specs.iter().any(|spec| spec.container == "gumgum-cadvisor"));
        assert!(
            specs
                .iter()
                .any(|spec| spec.container == "gumgum-docker-proxy")
        );
        assert!(specs.iter().all(|spec| !spec.container.contains("preview")));
        assert!(specs.iter().all(|spec| !spec.container.contains("prod")));
    }

    #[test]
    fn prometheus_config_includes_host_container_and_docker_discovery_jobs() {
        let config = prometheus_config_base();
        assert!(config.contains("job_name: gumgum-host"));
        assert!(config.contains("job_name: gumgum-containers"));
        assert!(config.contains("job_name: gumgum-docker"));
        assert!(config.contains("tcp://gumgum-docker-proxy:2375"));
        assert!(config.contains("__meta_docker_container_label_prometheus_scrape"));
    }

    #[test]
    fn alloy_config_ships_gumgum_docker_logs_to_loki() {
        let config = alloy_config();
        assert!(config.contains("loki.source.docker"));
        assert!(config.contains("loki.write"));
        assert!(config.contains("http://gumgum-loki:3100/loki/api/v1/push"));
        assert!(config.contains("__meta_docker_container_label_gumgum_managed"));
        assert!(config.contains("target_label  = \"environment\""));
        assert!(config.contains("target_label  = \"project\""));
        assert!(config.contains("target_label  = \"domain\""));
        assert!(config.contains("target_label  = \"worker\""));
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
        let labels = platform_labels(&spec(), "leostera.dev");
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
    fn grafana_folder_uid_builds_stable_nested_path() {
        let domain = grafana_folder_uid(None, "kava.fund");
        assert_eq!(domain, "gumgum-kava-fund");
        assert_eq!(
            grafana_folder_uid(Some(&domain), "visit-counter"),
            "gumgum-kava-fund-visit-counter"
        );
    }

    #[test]
    fn grafana_folder_create_existing_statuses_are_idempotent() {
        assert!(matches!(409, 409 | 412));
        assert!(matches!(412, 409 | 412));
    }

    #[test]
    fn grafana_dashboard_uid_is_stable_for_folder_scoped_names() {
        assert_eq!(
            grafana_dashboard_uid("visit-counter / API Overview", &["kava.fund".to_owned()]),
            "gumgum-kava-fund-visit-counter-api-overv"
        );
        assert_ne!(
            grafana_dashboard_uid("visit-counter / API Overview", &["kava.fund".to_owned()]),
            grafana_dashboard_uid(
                "visit-counter / API Overview",
                &["leostera.dev".to_owned(), "visit-counter".to_owned()]
            )
        );
    }

    #[test]
    fn grafana_path_escape_handles_datasource_names() {
        assert_eq!(
            grafana_path_escape("Project / Prometheus"),
            "Project%20%2F%20Prometheus"
        );
    }

    #[test]
    fn prometheus_platform_spec_mounts_config_and_enables_reload() {
        let prometheus = platform_specs("leostera.dev")
            .into_iter()
            .find(|spec| spec.container == "gumgum-prometheus")
            .unwrap();
        let spec = platform_run_spec(&prometheus, "leostera.dev");
        assert!(
            spec.binds
                .iter()
                .any(|bind| bind.ends_with(":/etc/prometheus/prometheus.yml:ro"))
        );
        assert!(
            !spec
                .binds
                .contains(&"/var/run/docker.sock:/var/run/docker.sock:ro".to_owned())
        );
        assert!(spec.command.contains(&"--web.enable-lifecycle".to_owned()));
    }

    #[test]
    fn platform_stateful_services_use_host_volume_paths() {
        let specs = platform_specs("leostera.dev");
        for (container, bind) in [
            (
                "gumgum-grafana",
                "/gumgum/volumes/platform/grafana:/var/lib/grafana",
            ),
            (
                "gumgum-prometheus",
                "/gumgum/volumes/platform/prometheus:/prometheus",
            ),
            ("gumgum-loki", "/gumgum/volumes/platform/loki:/loki"),
            ("gumgum-tempo", "/gumgum/volumes/platform/tempo:/tmp/tempo"),
            (
                "gumgum-alloy",
                "/gumgum/volumes/platform/alloy:/var/lib/alloy",
            ),
        ] {
            let provider = specs
                .iter()
                .find(|spec| spec.container == container)
                .unwrap();
            let spec = platform_run_spec(provider, "leostera.dev");
            assert!(spec.binds.contains(&bind.to_owned()));
        }
    }

    #[test]
    fn grafana_platform_labels_publish_route() {
        let grafana = platform_specs("leostera.dev")
            .into_iter()
            .find(|spec| spec.container == "gumgum-grafana")
            .unwrap();
        let labels = platform_labels(&grafana, "leostera.dev");
        assert_eq!(
            labels.get("caddy").map(String::as_str),
            Some("grafana.leostera.dev")
        );
        assert_eq!(
            labels.get("caddy.reverse_proxy").map(String::as_str),
            Some("{{upstreams 3000}}")
        );
        assert_eq!(
            labels.get("caddy.tls").map(String::as_str),
            Some("internal")
        );
    }
}
