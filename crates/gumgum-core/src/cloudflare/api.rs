use crate::{CloudflareGrant, ErrorCode, GumgumError, Result, Subsystem};
use reqwest::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const API_BASE: &str = "https://api.cloudflare.com/client/v4";
const CADDY_SERVICE: &str = "http://caddy-gateway:80";

#[derive(Clone)]
pub struct CloudflareClient {
    http: Client,
    token: String,
}

impl CloudflareClient {
    pub fn new(grant: &CloudflareGrant) -> Self {
        Self {
            http: Client::new(),
            token: grant.access_token.clone(),
        }
    }

    pub async fn ensure_route(&self, zone_name: &str, hostname: &str) -> Result<Vec<String>> {
        let zone = self.zone(zone_name).await?;
        let account_id = zone.account.id.clone();
        let tunnel = self.ensure_tunnel(&account_id, "gumgum").await?;
        self.ensure_tunnel_config(&account_id, &tunnel.id, hostname)
            .await?;
        self.upsert_cname(
            &zone.id,
            hostname,
            &format!("{}.cfargotunnel.com", tunnel.id),
        )
        .await?;
        Ok(vec![
            format!("ensure Cloudflare tunnel {}", tunnel.name),
            format!("ensure Cloudflare tunnel route {hostname} -> {CADDY_SERVICE}"),
            format!(
                "ensure Cloudflare DNS CNAME {hostname} -> {}.cfargotunnel.com",
                tunnel.id
            ),
        ])
    }

    async fn zone(&self, name: &str) -> Result<Zone> {
        let response: ListResponse<Zone> = self
            .get(&format!("/zones?name={}", url_encode(name)))
            .await?;
        response.result.into_iter().next().ok_or_else(|| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                format!("Cloudflare zone {name} was not found"),
            )
            .likely_cause("check that the API token is scoped to this zone")
            .build()
        })
    }

    async fn ensure_tunnel(&self, account_id: &str, name: &str) -> Result<Tunnel> {
        let existing: ListResponse<Tunnel> = self
            .get(&format!(
                "/accounts/{account_id}/cfd_tunnel?name={}",
                url_encode(name)
            ))
            .await?;
        if let Some(tunnel) = existing.result.into_iter().find(|tunnel| !tunnel.deleted) {
            return Ok(tunnel);
        }
        self.post(
            &format!("/accounts/{account_id}/cfd_tunnel"),
            &serde_json::json!({ "name": name, "config_src": "cloudflare" }),
        )
        .await
    }

    async fn ensure_tunnel_config(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
    ) -> Result<()> {
        let _: CloudflareEnvelope<serde_json::Value> = self
            .put_raw(
                &format!("/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations"),
                &serde_json::json!({
                    "config": {
                        "ingress": [
                            { "hostname": hostname, "service": CADDY_SERVICE },
                            { "service": "http_status:404" }
                        ]
                    }
                }),
            )
            .await?;
        Ok(())
    }

    async fn upsert_cname(&self, zone_id: &str, hostname: &str, target: &str) -> Result<()> {
        let existing: ListResponse<DnsRecord> = self
            .get(&format!(
                "/zones/{zone_id}/dns_records?type=CNAME&name={}",
                url_encode(hostname)
            ))
            .await?;
        let body = serde_json::json!({
            "type": "CNAME",
            "name": hostname,
            "content": target,
            "proxied": true,
            "ttl": 1
        });
        if let Some(record) = existing.result.into_iter().next() {
            let _: DnsRecord = self
                .put(
                    &format!("/zones/{zone_id}/dns_records/{}", record.id),
                    &body,
                )
                .await?;
        } else {
            let _: DnsRecord = self
                .post(&format!("/zones/{zone_id}/dns_records"), &body)
                .await?;
        }
        Ok(())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.decode(self.http.get(format!("{API_BASE}{path}")))
            .await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        self.decode(self.http.post(format!("{API_BASE}{path}")).json(body))
            .await
    }

    async fn put<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        self.decode(self.http.put(format!("{API_BASE}{path}")).json(body))
            .await
    }

    async fn put_raw<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        self.decode_raw(self.http.put(format!("{API_BASE}{path}")).json(body))
            .await
    }

    async fn decode<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> Result<T> {
        Ok(self
            .decode_raw::<CloudflareEnvelope<T>>(request)
            .await?
            .result)
    }

    async fn decode_raw<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> Result<T> {
        request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|source| cf_error("Cloudflare API request failed", source))?
            .error_for_status()
            .map_err(|source| cf_error("Cloudflare API returned an error", source))?
            .json()
            .await
            .map_err(|source| cf_error("could not decode Cloudflare API response", source))
    }
}

#[derive(Debug, Deserialize)]
struct CloudflareEnvelope<T> {
    result: T,
}

#[derive(Debug, Deserialize)]
struct ListResponse<T> {
    result: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct Zone {
    id: String,
    account: Account,
}

#[derive(Debug, Deserialize)]
struct Account {
    id: String,
}

#[derive(Debug, Deserialize)]
struct Tunnel {
    id: String,
    name: String,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct DnsRecord {
    id: String,
}

fn cf_error(message: &str, source: reqwest::Error) -> GumgumError {
    GumgumError::structured(Subsystem::Config, ErrorCode::Io, message)
        .likely_cause(source.to_string())
        .build()
}

fn url_encode(value: &str) -> String {
    value.replace('.', "%2E")
}
