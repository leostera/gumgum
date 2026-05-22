use crate::{ConfigStore, ErrorCode, GumgumError, Result, Subsystem};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::{Rng, distributions::Alphanumeric};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, process::Command};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::types::{CLOUDFLARE_PROVIDER, CloudflareGrant, CloudflareTokenResponse};

const AUTH_URL: &str = "https://dash.cloudflare.com/oauth2/auth";
const TOKEN_URL: &str = "https://dash.cloudflare.com/oauth2/token";
const WRANGLER_CLIENT_ID: &str = "54d11594-84e4-41aa-b438-e81b8faaee7f";
const DEFAULT_SCOPES: &[&str] = &[
    "com.cloudflare.api.account.zone.list",
    "com.cloudflare.api.account.account_settings.read",
    "com.cloudflare.api.account.dns_records.read",
    "com.cloudflare.api.account.dns_records.edit",
    "com.cloudflare.api.account.cloudflare_tunnel.read",
    "com.cloudflare.api.account.cloudflare_tunnel.edit",
];

pub async fn ensure_authorized_for_zone(
    store: &ConfigStore,
    zone_name: &str,
    interactive: bool,
) -> Result<CloudflareGrant> {
    if let Some(grant) = store.load_cloudflare_grant()? {
        if grant.zone_name == zone_name {
            return Ok(grant);
        }
    }
    if !interactive {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            format!("Cloudflare authorization required for {zone_name}"),
        )
        .likely_cause("cloudflare ingress needs an interactive browser authorization")
        .next_command("rerun without --json or --dry-run in an interactive terminal")
        .build());
    }
    let grant = authorize_zone(zone_name).await?;
    store.save_cloudflare_grant(&grant)?;
    Ok(grant)
}

pub async fn authorize_zone(zone_name: &str) -> Result<CloudflareGrant> {
    let client_id = cloudflare_client_id()?;
    let (verifier, challenge) = generate_pkce();
    let state = random_string(32);
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not start Cloudflare OAuth callback listener",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let port = listener
        .local_addr()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not read callback port",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/cloudflare/callback");
    let scopes = DEFAULT_SCOPES.join(" ");
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        enc(&client_id),
        enc(&redirect_uri),
        enc(&scopes),
        enc(&state),
        enc(&challenge),
    );
    eprintln!("Cloudflare authorization required for {zone_name}.");
    eprintln!("Opening browser to authorize GumGum for this zone...");
    if open_browser(&url).is_err() {
        eprintln!("Open this URL in your browser:\n{url}");
    }
    let params = wait_for_callback(listener).await?;
    if params.get("state") != Some(&state) {
        return Err(GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            "Cloudflare OAuth state did not match",
        )
        .build());
    }
    let code = params.get("code").ok_or_else(|| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::InvalidArgs,
            "Cloudflare OAuth callback did not include a code",
        )
        .build()
    })?;
    let token = exchange_code(&client_id, &redirect_uri, code, &verifier).await?;
    Ok(CloudflareGrant {
        account_id: None,
        zone_id: None,
        zone_name: zone_name.to_owned(),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_in: token.expires_in,
        scopes: token
            .scope
            .unwrap_or(scopes)
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect(),
    })
}

async fn wait_for_callback(listener: TcpListener) -> Result<HashMap<String, String>> {
    let (mut stream, _) = listener.accept().await.map_err(|source| {
        GumgumError::structured(Subsystem::Config, ErrorCode::Io, "OAuth callback failed")
            .likely_cause(source.to_string())
            .build()
    })?;
    let mut buffer = vec![0; 8192];
    let read = stream.read(&mut buffer).await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not read OAuth callback",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    let params = parse_query(
        path.split_once('?')
            .map(|(_, query)| query)
            .unwrap_or_default(),
    );
    let body = "Cloudflare authorization complete. You can return to GumGum.";
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    Ok(params)
}

async fn exchange_code(
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<CloudflareTokenResponse> {
    let body = format!(
        "grant_type=authorization_code&client_id={}&redirect_uri={}&code={}&code_verifier={}",
        enc(client_id),
        enc(redirect_uri),
        enc(code),
        enc(verifier)
    );
    Client::new()
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "Cloudflare token exchange failed",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .error_for_status()
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "Cloudflare rejected OAuth token exchange",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .json()
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not decode Cloudflare token response",
            )
            .likely_cause(source.to_string())
            .build()
        })
}

fn generate_pkce() -> (String, String) {
    let verifier = random_string(64);
    let challenge = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        Sha256::digest(verifier.as_bytes()),
    );
    (verifier, challenge)
}

fn random_string(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn cloudflare_client_id() -> Result<String> {
    Ok(std::env::var("GUMGUM_CLOUDFLARE_CLIENT_ID")
        .unwrap_or_else(|_| WRANGLER_CLIENT_ID.to_owned()))
}

fn open_browser(url: &str) -> std::io::Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()?
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", url]).status()?
    } else {
        Command::new("xdg-open").arg(url).status()?
    };
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("browser open failed"))
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.replace('+', " ")))
        .collect()
}

fn enc(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

#[allow(dead_code)]
fn _provider_name() -> &'static str {
    CLOUDFLARE_PROVIDER
}
