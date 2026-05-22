use crate::{CloudflareGrant, ErrorCode, GumgumError, Result, Subsystem};
use reqwest::Client;
use serde::Deserialize;

const API_BASE: &str = "https://api.cloudflare.com/client/v4";
const CADDY_SERVICE: &str = "https://gumgum-caddy:443";

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

    pub async fn ensure_route(&self, zone_name: &str, hostname: &str) -> Result<CloudflareRoute> {
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
        let tunnel_token = self.tunnel_token(&account_id, &tunnel.id).await?;
        Ok(CloudflareRoute {
            tunnel_token,
            actions: vec![
                format!("ensure Cloudflare tunnel {}", tunnel.name),
                format!("ensure Cloudflare tunnel route {hostname} -> {CADDY_SERVICE}"),
                format!(
                    "ensure Cloudflare DNS CNAME {hostname} -> {}.cfargotunnel.com",
                    tunnel.id
                ),
            ],
        })
    }

    async fn zone(&self, name: &str) -> Result<Zone> {
        let response = self
            .get_json(&format!("/zones?name={}", url_encode(name)))
            .await?;
        result_array(&response)
            .and_then(|zones| zones.first().cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| {
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
        let existing = self
            .get_json(&format!(
                "/accounts/{account_id}/cfd_tunnel?name={}",
                url_encode(name)
            ))
            .await?;
        if let Some(tunnel) = result_array(&existing).and_then(|tunnels| {
            tunnels.into_iter().find_map(|value| {
                let tunnel: Tunnel = serde_json::from_value(value).ok()?;
                if tunnel.deleted_at.is_none() && !tunnel.deleted.unwrap_or(false) {
                    Some(tunnel)
                } else {
                    None
                }
            })
        }) {
            return Ok(tunnel);
        }
        let created = self
            .post_json(
                &format!("/accounts/{account_id}/cfd_tunnel"),
                &serde_json::json!({ "name": name, "config_src": "cloudflare" }),
            )
            .await?;
        serde_json::from_value(result_value(&created)?.clone()).map_err(|source| {
            cf_message_error(
                "could not decode Cloudflare tunnel create response",
                source.to_string(),
            )
        })
    }

    async fn ensure_tunnel_config(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
    ) -> Result<()> {
        self.put_json(
            &format!("/accounts/{account_id}/cfd_tunnel/{tunnel_id}/configurations"),
            &serde_json::json!({
                "config": {
                    "ingress": [
                        {
                            "hostname": hostname,
                            "service": CADDY_SERVICE,
                            "originRequest": {
                                "noTLSVerify": true,
                                "originServerName": hostname
                            }
                        },
                        { "service": "http_status:404" }
                    ]
                }
            }),
        )
        .await?;
        Ok(())
    }

    async fn tunnel_token(&self, account_id: &str, tunnel_id: &str) -> Result<String> {
        let response = self
            .get_json(&format!(
                "/accounts/{account_id}/cfd_tunnel/{tunnel_id}/token"
            ))
            .await?;
        let result = result_value(&response)?;
        if let Some(token) = result.as_str() {
            return Ok(token.to_owned());
        }
        if let Some(token) = result.get("token").and_then(|value| value.as_str()) {
            return Ok(token.to_owned());
        }
        Err(cf_message_error(
            "could not decode Cloudflare tunnel token response",
            result.to_string(),
        ))
    }

    pub async fn delete_route_dns(&self, zone_name: &str, hostname: &str) -> Result<Vec<String>> {
        let zone = self.zone(zone_name).await?;
        match self.delete_cname(&zone.id, hostname).await? {
            DeleteCnameResult::Deleted => {
                Ok(vec![format!("delete Cloudflare DNS CNAME {hostname}")])
            }
            DeleteCnameResult::Absent => Ok(vec![format!(
                "Cloudflare DNS CNAME {hostname} was already absent"
            )]),
            DeleteCnameResult::Unmanaged => Ok(vec![format!(
                "Cloudflare DNS CNAME {hostname} was not deleted because it is not marked managed-by=gumgum"
            )]),
        }
    }

    async fn upsert_cname(&self, zone_id: &str, hostname: &str, target: &str) -> Result<()> {
        let existing = self
            .get_json(&format!(
                "/zones/{zone_id}/dns_records?type=CNAME&name={}",
                url_encode(hostname)
            ))
            .await?;
        let body = serde_json::json!({
            "type": "CNAME",
            "name": hostname,
            "content": target,
            "proxied": true,
            "ttl": 1,
            "comment": "managed-by=gumgum"
        });
        if let Some(record_id) = result_array(&existing).and_then(|records| {
            records
                .first()
                .and_then(|record| record.get("id"))
                .and_then(|id| id.as_str())
                .map(ToOwned::to_owned)
        }) {
            self.put_json(&format!("/zones/{zone_id}/dns_records/{record_id}"), &body)
                .await?;
        } else {
            self.post_json(&format!("/zones/{zone_id}/dns_records"), &body)
                .await?;
        }
        Ok(())
    }

    async fn delete_cname(&self, zone_id: &str, hostname: &str) -> Result<DeleteCnameResult> {
        let existing = self
            .get_json(&format!(
                "/zones/{zone_id}/dns_records?type=CNAME&name={}",
                url_encode(hostname)
            ))
            .await?;
        let Some(record) = result_array(&existing).and_then(|records| records.first().cloned())
        else {
            return Ok(DeleteCnameResult::Absent);
        };
        let managed = record
            .get("comment")
            .and_then(|comment| comment.as_str())
            .is_some_and(|comment| comment == "managed-by=gumgum");
        if !managed {
            return Ok(DeleteCnameResult::Unmanaged);
        }
        let Some(record_id) = record.get("id").and_then(|id| id.as_str()) else {
            return Ok(DeleteCnameResult::Absent);
        };
        self.delete_json(&format!("/zones/{zone_id}/dns_records/{record_id}"))
            .await?;
        Ok(DeleteCnameResult::Deleted)
    }

    async fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        self.decode_json(self.http.get(format!("{API_BASE}{path}")))
            .await
    }

    async fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.decode_json(self.http.post(format!("{API_BASE}{path}")).json(body))
            .await
    }

    async fn put_json(&self, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.decode_json(self.http.put(format!("{API_BASE}{path}")).json(body))
            .await
    }

    async fn delete_json(&self, path: &str) -> Result<serde_json::Value> {
        self.decode_json(self.http.delete(format!("{API_BASE}{path}")))
            .await
    }

    async fn decode_json(&self, request: reqwest::RequestBuilder) -> Result<serde_json::Value> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|source| cf_error("Cloudflare API request failed", source))?
            .error_for_status()
            .map_err(|source| cf_error("Cloudflare API returned an error", source))?;
        let body = response
            .text()
            .await
            .map_err(|source| cf_error("could not read Cloudflare API response body", source))?;
        serde_json::from_str(&body).map_err(|source| {
            cf_message_error(
                "could not decode Cloudflare API response",
                format!(
                    "{source}; body: {}",
                    body.chars().take(500).collect::<String>()
                ),
            )
        })
    }
}

pub struct CloudflareRoute {
    pub actions: Vec<String>,
    pub tunnel_token: String,
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
    deleted: Option<bool>,
    #[serde(default)]
    deleted_at: Option<serde_json::Value>,
}

enum DeleteCnameResult {
    Deleted,
    Absent,
    Unmanaged,
}

fn result_value(response: &serde_json::Value) -> Result<&serde_json::Value> {
    response.get("result").ok_or_else(|| {
        cf_message_error(
            "Cloudflare API response did not include a result",
            response.to_string(),
        )
    })
}

fn result_array(response: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    response.get("result")?.as_array().cloned()
}

fn cf_error(message: &str, source: reqwest::Error) -> GumgumError {
    GumgumError::structured(Subsystem::Config, ErrorCode::Io, message)
        .likely_cause(source.to_string())
        .build()
}

fn cf_message_error(message: &str, cause: String) -> GumgumError {
    GumgumError::structured(Subsystem::Config, ErrorCode::Io, message)
        .likely_cause(cause)
        .build()
}

fn url_encode(value: &str) -> String {
    value.replace('.', "%2E")
}
