use crate::{Capability, ErrorCode, GumgumError, Subsystem, sanitize_name};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use sha2::{Digest, Sha256};
use tokio::process::Command as TokioCommand;

type HmacSha256 = Hmac<Sha256>;
const PATH_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

use super::docker::{
    create_provider_container, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ObjectProviderPlan, ProviderCredentials, ProviderSpec};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Blob,
        provider: "minio.main".to_owned(),
        container: "gumgum-provider-minio-main".to_owned(),
        image: "minio/minio:latest".to_owned(),
        port: 9000,
        protocol: "s3".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> Vec<String> {
    vec![
        "ensure minio.main provider is running".to_owned(),
        format!("ensure bucket {safe_name} exists"),
        format!("publish DNS {dns} to minio.main"),
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<String> {
    vec![
        format!("aws --endpoint-url http://{dns}:9000 s3 mb s3://{name}"),
        format!("S3_ENDPOINT=http://{dns}:9000 S3_BUCKET={name}"),
    ]
}

pub(crate) async fn ensure(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    ensure_bucket(&bucket, &credentials).await?;
    actions.push(format!("ensured bucket {bucket} on {}", provider.provider));
    actions.push(format!("published DNS {} to minio.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn delete(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let provider = &plan.provider;
    let mut actions = ensure_provider(provider, credentials.clone()).await?;
    let bucket = sanitize_name(&plan.name);
    delete_bucket(&bucket, &credentials).await?;
    actions.push(format!(
        "deleted bucket {bucket} from {}",
        provider.provider
    ));
    actions.push(format!("removed DNS {} from minio.main", plan.dns));
    Ok(actions)
}

pub(crate) async fn ensure_provider(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start minio provider").await;
    }
    create_provider_container(
        provider,
        vec![
            (credentials.username_env, credentials.username),
            (credentials.password_env, credentials.password),
        ],
        vec![
            "server".to_owned(),
            "/data".to_owned(),
            "--console-address".to_owned(),
            ":9001".to_owned(),
        ],
    )
    .await
}

async fn ensure_bucket(bucket: &str, credentials: &ProviderCredentials) -> crate::Result<()> {
    let script = format!(
        "set -e; mc alias set gumgum-minio http://gumgum-provider-minio-main:9000 '{}' '{}'; mc mb --ignore-existing gumgum-minio/{}",
        shell_single_quote(&credentials.username),
        shell_single_quote(&credentials.password),
        shell_single_quote(bucket)
    );
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("gumgum-network")
            .arg("--entrypoint")
            .arg("/bin/sh")
            .arg("minio/mc:latest")
            .arg("-c")
            .arg(script),
        "could not ensure minio bucket",
    )
    .await
}

async fn delete_bucket(bucket: &str, credentials: &ProviderCredentials) -> crate::Result<()> {
    let script = format!(
        "set -e; mc alias set gumgum-minio http://gumgum-provider-minio-main:9000 '{}' '{}'; mc rb --force gumgum-minio/{} || true",
        shell_single_quote(&credentials.username),
        shell_single_quote(&credentials.password),
        shell_single_quote(bucket)
    );
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("gumgum-network")
            .arg("--entrypoint")
            .arg("/bin/sh")
            .arg("minio/mc:latest")
            .arg("-c")
            .arg(script),
        "could not delete minio bucket",
    )
    .await
}

pub async fn list_objects(
    bucket: &str,
    path: Option<&str>,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let bucket = sanitize_name(bucket);
    let prefix = path.unwrap_or_default().trim_start_matches('/');
    let endpoint = minio_endpoint().await?;
    let query = if prefix.is_empty() {
        "list-type=2".to_owned()
    } else {
        format!("list-type=2&prefix={}", percent(prefix))
    };
    let response = s3_request(S3Request {
        method: reqwest::Method::GET,
        endpoint: &endpoint,
        bucket: &bucket,
        key: "",
        query: Some(&query),
        credentials,
        body: Vec::new(),
        extra_headers: Vec::new(),
    })
    .await?;
    let body = response
        .text()
        .await
        .map_err(|source| s3_error("could not read minio list response", source))?;
    Ok(body
        .split("<Key>")
        .skip(1)
        .filter_map(|chunk| chunk.split_once("</Key>").map(|(key, _)| key.to_owned()))
        .collect())
}

pub async fn get_object_bytes(
    bucket: &str,
    path: &str,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<u8>> {
    let bucket = sanitize_name(bucket);
    let endpoint = minio_endpoint().await?;
    let response = s3_request(S3Request {
        method: reqwest::Method::GET,
        endpoint: &endpoint,
        bucket: &bucket,
        key: path,
        query: None,
        credentials,
        body: Vec::new(),
        extra_headers: Vec::new(),
    })
    .await?;
    Ok(response
        .bytes()
        .await
        .map_err(|source| s3_error("could not read minio bucket object", source))?
        .to_vec())
}

pub async fn get_object(
    bucket: &str,
    path: &str,
    credentials: &ProviderCredentials,
) -> crate::Result<String> {
    let bytes = get_object_bytes(bucket, path, credentials).await?;
    String::from_utf8(bytes).map_err(|source| {
        GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::InvalidArgs,
            "minio object is not valid UTF-8",
        )
        .likely_cause(source.to_string())
        .build()
    })
}

pub async fn put_object(
    bucket: &str,
    path: &str,
    content: Vec<u8>,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let bucket = sanitize_name(bucket);
    let endpoint = minio_endpoint().await?;
    s3_request(S3Request {
        method: reqwest::Method::PUT,
        endpoint: &endpoint,
        bucket: &bucket,
        key: path,
        query: None,
        credentials,
        body: content,
        extra_headers: Vec::new(),
    })
    .await?;
    Ok(vec![format!("uploaded {bucket}/{path} to minio.main")])
}

pub async fn remove_object(
    bucket: &str,
    path: &str,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let bucket = sanitize_name(bucket);
    let endpoint = minio_endpoint().await?;
    s3_request(S3Request {
        method: reqwest::Method::DELETE,
        endpoint: &endpoint,
        bucket: &bucket,
        key: path,
        query: None,
        credentials,
        body: Vec::new(),
        extra_headers: Vec::new(),
    })
    .await?;
    Ok(vec![format!("removed {bucket}/{path} from minio.main")])
}

pub async fn copy_object(
    source: &str,
    destination: &str,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let (source_bucket, source_key) = split_remote_object(source)?;
    let (destination_bucket, destination_key) = split_remote_object(destination)?;
    let endpoint = minio_endpoint().await?;
    let destination_bucket = sanitize_name(&destination_bucket);
    s3_request(S3Request {
        method: reqwest::Method::PUT,
        endpoint: &endpoint,
        bucket: &destination_bucket,
        key: &destination_key,
        query: None,
        credentials,
        body: Vec::new(),
        extra_headers: vec![(
            "x-amz-copy-source".to_owned(),
            format!("/{}/{}", sanitize_name(&source_bucket), source_key),
        )],
    })
    .await?;
    Ok(vec![format!(
        "copied {source} to {destination} in minio.main"
    )])
}

pub async fn sync_objects(
    source: &str,
    destination: &str,
    credentials: &ProviderCredentials,
) -> crate::Result<Vec<String>> {
    let (source_bucket, source_prefix) = split_remote_object(source)?;
    let objects = list_objects(&source_bucket, Some(&source_prefix), credentials).await?;
    let (_, destination_prefix) = split_remote_object(destination)?;
    for object in &objects {
        let suffix = object
            .strip_prefix(&source_prefix)
            .unwrap_or(object)
            .trim_start_matches('/');
        let target = format!("{}/{}", destination.trim_end_matches('/'), suffix);
        copy_object(&format!("{source_bucket}/{object}"), &target, credentials).await?;
        let _ = &destination_prefix;
    }
    Ok(vec![format!(
        "synced {source} to {destination} in minio.main"
    )])
}

async fn minio_endpoint() -> crate::Result<String> {
    let output = TokioCommand::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg("{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}")
        .arg("gumgum-provider-minio-main")
        .output()
        .await
        .map_err(|source| s3_error("could not inspect minio provider container", source))?;
    if !output.status.success() {
        return Err(GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "could not inspect minio provider container",
        )
        .likely_cause(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        .build());
    }
    let ip = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if ip.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "minio provider container has no Docker network address",
        )
        .build());
    }
    Ok(format!("http://{ip}:9000"))
}

struct S3Request<'a> {
    method: reqwest::Method,
    endpoint: &'a str,
    bucket: &'a str,
    key: &'a str,
    query: Option<&'a str>,
    credentials: &'a ProviderCredentials,
    body: Vec<u8>,
    extra_headers: Vec<(String, String)>,
}

async fn s3_request(request: S3Request<'_>) -> crate::Result<reqwest::Response> {
    let now = time::OffsetDateTime::now_utc();
    let amz_date = now
        .format(
            &time::format_description::parse("[year][month][day]T[hour][minute][second]Z")
                .expect("valid amz date format"),
        )
        .expect("format amz date");
    let date = &amz_date[..8];
    let canonical_uri = if request.key.is_empty() {
        format!("/{}/", request.bucket)
    } else {
        format!("/{}/{}", request.bucket, percent_path(request.key))
    };
    let canonical_query = request.query.unwrap_or_default();
    let host = request
        .endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let payload_hash = hex::encode(Sha256::digest(&request.body));
    let mut headers = vec![
        ("host".to_owned(), host.to_owned()),
        ("x-amz-content-sha256".to_owned(), payload_hash.clone()),
        ("x-amz-date".to_owned(), amz_date.clone()),
    ];
    headers.extend(
        request
            .extra_headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value)),
    );
    headers.sort_by(|left, right| left.0.cmp(&right.0));
    let canonical_headers = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect::<String>();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method.as_str(),
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        payload_hash
    );
    let scope = format!("{date}/us-east-1/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let signing_key = signing_key(&request.credentials.password, date);
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        request.credentials.username
    );
    let url = if canonical_query.is_empty() {
        format!("{}{}", request.endpoint, canonical_uri)
    } else {
        format!("{}{}?{canonical_query}", request.endpoint, canonical_uri)
    };
    let client = reqwest::Client::new();
    let mut builder = client.request(request.method, url).body(request.body);
    for (name, value) in headers {
        builder = builder.header(&name, value);
    }
    let response = builder
        .header("authorization", authorization)
        .send()
        .await
        .map_err(|source| s3_error("could not call minio S3 API", source))?;
    if response.status().is_success() {
        Ok(response)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(GumgumError::structured(
            Subsystem::Setup,
            ErrorCode::Io,
            "minio S3 API returned an error",
        )
        .likely_cause(format!("{status}: {text}"))
        .build())
    }
}

fn signing_key(secret: &str, date: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, b"us-east-1");
    let k_service = hmac_sha256(&k_region, b"s3");
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn percent(value: &str) -> String {
    utf8_percent_encode(value, PATH_ENCODE_SET).to_string()
}

fn percent_path(value: &str) -> String {
    value.split('/').map(percent).collect::<Vec<_>>().join("/")
}

fn split_remote_object(value: &str) -> crate::Result<(String, String)> {
    let (bucket, key) = value
        .trim_start_matches('/')
        .split_once('/')
        .ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Cli,
                ErrorCode::InvalidArgs,
                format!("bucket object path must be bucket/key: {value}"),
            )
            .build()
        })?;
    if bucket.is_empty() || key.is_empty() {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            format!("bucket object path must be bucket/key: {value}"),
        )
        .build());
    }
    Ok((bucket.to_owned(), key.to_owned()))
}

fn s3_error(message: &str, source: impl ToString) -> GumgumError {
    GumgumError::structured(Subsystem::Setup, ErrorCode::Io, message)
        .likely_cause(source.to_string())
        .build()
}

pub(crate) fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}
