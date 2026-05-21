use crate::{Capability, sanitize_name};

use super::types::{ObjectProviderPlan, ProviderSpec};

pub fn provider_spec(capability: Capability) -> ProviderSpec {
    match capability {
        Capability::Db => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-postgres-main".to_owned(),
            image: "postgres:16-alpine".to_owned(),
            port: 5432,
            protocol: "postgres".to_owned(),
        },
        Capability::Kv => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-redis-main".to_owned(),
            image: "redis:7-alpine".to_owned(),
            port: 6379,
            protocol: "redis".to_owned(),
        },
        Capability::Blob => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-minio-main".to_owned(),
            image: "minio/minio:latest".to_owned(),
            port: 9000,
            protocol: "s3".to_owned(),
        },
        Capability::Queue => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-redpanda-main".to_owned(),
            image: "redpandadata/redpanda:latest".to_owned(),
            port: 9092,
            protocol: "kafka".to_owned(),
        },
        Capability::Secret => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-onepassword-main".to_owned(),
            image: "1password/connect-api:latest".to_owned(),
            port: 8080,
            protocol: "onepassword-connect".to_owned(),
        },
        Capability::Observability => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-otel-platform".to_owned(),
            image: "otel/opentelemetry-collector-contrib:latest".to_owned(),
            port: 4317,
            protocol: "otlp".to_owned(),
        },
        Capability::Manual => ProviderSpec {
            capability,
            provider: capability.provider().to_owned(),
            container: "gumgum-provider-manual-main".to_owned(),
            image: "manual".to_owned(),
            port: 0,
            protocol: "manual".to_owned(),
        },
    }
}

pub fn vaultwarden_spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Secret,
        provider: "vaultwarden.main".to_owned(),
        container: "gumgum-provider-vaultwarden-main".to_owned(),
        image: "vaultwarden/server:latest".to_owned(),
        port: 80,
        protocol: "bitwarden-compatible".to_owned(),
    }
}

pub fn object_provider_plan(capability: Capability, name: &str, dns: &str) -> ObjectProviderPlan {
    let provider = provider_spec(capability);
    let safe_name = sanitize_name(name);
    ObjectProviderPlan {
        capability,
        name: name.to_owned(),
        dns: dns.to_owned(),
        actions: provider_actions(capability, &safe_name, dns),
        connection_examples: connection_examples(capability, name, dns),
        provider,
    }
}

pub fn connection_examples(capability: Capability, name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => vec![
            format!("psql postgres://{name}:<password>@{dns}:5432/{name}"),
            format!("pgAdmin host={dns} port=5432 database={name} username={name}"),
        ],
        Capability::Kv => vec![
            format!("redis-cli -u redis://{dns}:6379/0"),
            format!("RedisInsight host={dns} port=6379 database=0"),
        ],
        Capability::Blob => vec![
            format!("aws --endpoint-url http://{dns}:9000 s3 mb s3://{name}"),
            format!("S3_ENDPOINT=http://{dns}:9000 S3_BUCKET={name}"),
        ],
        Capability::Queue => vec![
            format!("kcat -b {dns}:9092 -t {name}"),
            format!("KAFKA_BROKERS={dns}:9092 KAFKA_TOPIC={name}"),
        ],
        Capability::Secret => vec![
            format!("op item get {name} --vault gumgum"),
            format!("onepassword://gumgum/{name}"),
        ],
        Capability::Observability => vec![format!("OTEL_EXPORTER_OTLP_ENDPOINT=http://{dns}:4317")],
        Capability::Manual => Vec::new(),
    }
}

fn provider_actions(capability: Capability, safe_name: &str, dns: &str) -> Vec<String> {
    match capability {
        Capability::Db => vec![
            "ensure postgres.main provider is running".to_owned(),
            format!("ensure database {safe_name} exists"),
            format!("publish DNS {dns} to postgres.main"),
        ],
        Capability::Kv => vec![
            "ensure redis.main provider is running".to_owned(),
            format!("reserve Redis key prefix {safe_name}:"),
            format!("publish DNS {dns} to redis.main"),
        ],
        Capability::Blob => vec![
            "ensure minio.main provider is running".to_owned(),
            format!("ensure bucket {safe_name} exists"),
            format!("publish DNS {dns} to minio.main"),
        ],
        Capability::Queue => vec![
            "ensure redpanda.main provider is running".to_owned(),
            format!("ensure topic {safe_name} exists"),
            format!("publish DNS {dns} to redpanda.main"),
        ],
        Capability::Secret => vec![
            "configure a secret provider such as vaultwarden.main".to_owned(),
            format!("map secret {safe_name} from the configured secret provider"),
            "do not materialize secret values in the graph".to_owned(),
        ],
        Capability::Observability => vec![
            "ensure otel.platform provider is running".to_owned(),
            format!("publish DNS {dns} to otel.platform"),
        ],
        Capability::Manual => {
            vec!["manual provider requires operator-managed backing service".to_owned()]
        }
    }
}
