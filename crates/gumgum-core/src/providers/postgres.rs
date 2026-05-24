use super::docker::{
    create_provider_container, ensure_network, inspect, provider_needs_recreate, start_existing,
};
use crate::{
    Capability, CoreAction, CoreActions, DockerEngine, ErrorCode, ErrorKind, GumgumError,
    Subsystem, sanitize_name,
};

use super::types::{ObjectProviderPlan, ProviderCredentials, ProviderSpec};

pub fn spec() -> ProviderSpec {
    ProviderSpec {
        capability: Capability::Db,
        provider: "postgres.main".to_owned(),
        container: "gumgum-provider-postgres-main".to_owned(),
        image: "postgres:16-alpine".to_owned(),
        port: 5432,
        protocol: "postgres".to_owned(),
    }
}

pub(crate) fn actions(safe_name: &str, dns: &str) -> CoreActions {
    vec![
        CoreAction::ProviderConfigured {
            capability: Capability::Db,
            provider: "postgres.main".to_owned(),
        },
        CoreAction::DatabaseCreated {
            database: safe_name.to_owned(),
        },
        CoreAction::DnsPublished {
            dns: dns.to_owned(),
            provider: "postgres.main".to_owned(),
        },
    ]
}

pub(crate) fn connection_examples(name: &str, dns: &str) -> Vec<crate::ConnectionExample> {
    vec![
        crate::ConnectionExample::PostgresPsql {
            name: name.to_owned(),
            dns: dns.to_owned(),
        },
        crate::ConnectionExample::PgAdmin {
            name: name.to_owned(),
            dns: dns.to_owned(),
        },
    ]
}

pub(crate) async fn ensure_object(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    let mut actions = ensure(&plan.provider, credentials.clone()).await?;
    let database = sanitize_name(&plan.name);
    let owner = sanitize_name(&plan.name);
    if let Some(password) = &plan.object_password {
        ensure_role(&plan.provider, &credentials, &owner, password).await?;
        actions.push(CoreAction::DatabaseRoleEnsured {
            role: owner.clone(),
        });
    }
    if database_exists(&plan.provider, &credentials, &database).await? {
        actions.push(CoreAction::DatabaseAlreadyExists {
            database: database.clone(),
        });
    } else {
        create_database(
            &plan.provider,
            &credentials,
            &database,
            plan.object_password.as_deref().map(|_| owner.as_str()),
        )
        .await?;
        actions.push(CoreAction::DatabaseCreated {
            database: database.clone(),
        });
    }
    if plan.object_password.is_some() {
        grant_database(&plan.provider, &credentials, &database, &owner).await?;
        actions.push(CoreAction::DatabaseGranted {
            database: database.clone(),
            role: owner.clone(),
        });
    }
    actions.push(CoreAction::DnsPublished {
        dns: plan.dns.clone(),
        provider: "postgres.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn delete_object(
    plan: &ObjectProviderPlan,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    let mut actions = ensure(&plan.provider, credentials.clone()).await?;
    let database = sanitize_name(&plan.name);
    if database_exists(&plan.provider, &credentials, &database).await? {
        drop_database(&plan.provider, &credentials, &database).await?;
        actions.push(CoreAction::DatabaseDropped {
            database: database.clone(),
        });
    } else {
        actions.push(CoreAction::DatabaseAlreadyAbsent {
            database: database.clone(),
        });
    }
    actions.push(CoreAction::DnsRemoved {
        dns: plan.dns.clone(),
        provider: "postgres.main".to_owned(),
    });
    Ok(actions)
}

pub(crate) async fn ensure(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<CoreActions> {
    ensure_network().await?;
    let actions = if inspect(&provider.container).await && !provider_needs_recreate(provider).await
    {
        start_existing(provider, "could not start postgres provider").await?
    } else {
        if inspect(&provider.container).await {
            DockerEngine::local()?
                .remove_container_force(&provider.container)
                .await?;
        }
        create_provider_container(
            provider,
            vec![
                (
                    credentials.username_env.clone(),
                    credentials.username.clone(),
                ),
                (
                    credentials.password_env.clone(),
                    credentials.password.clone(),
                ),
            ],
            Vec::new(),
        )
        .await?
    };
    wait_for_postgres(provider, &credentials).await?;
    Ok(actions)
}

async fn wait_for_postgres(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
) -> crate::Result<()> {
    let mut last_error = None;
    let mut consecutive_successes = 0;
    for _ in 0..30 {
        match DockerEngine::local()?
            .exec_success(
                &provider.container,
                vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
                vec![
                    "psql".to_owned(),
                    "-U".to_owned(),
                    credentials.username.clone(),
                    "-d".to_owned(),
                    "postgres".to_owned(),
                    "-tAc".to_owned(),
                    "SELECT 1".to_owned(),
                ],
            )
            .await
        {
            Ok(output) if output.trim() == "1" => {
                consecutive_successes += 1;
                if consecutive_successes >= 2 {
                    return Ok(());
                }
            }
            Ok(output) => {
                consecutive_successes = 0;
                last_error = Some(output);
            }
            Err(error) => {
                consecutive_successes = 0;
                last_error = Some(error.to_report().message);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(GumgumError::structured_kind(
        Subsystem::Setup,
        ErrorCode::Io,
        ErrorKind::PostgresProviderReadinessFailed,
    )
    .likely_cause(last_error.unwrap_or_else(|| "readiness check timed out".to_owned()))
    .build())
}

async fn database_exists(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
) -> crate::Result<bool> {
    let output = DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "psql".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                "-tAc".to_owned(),
                format!(
                    "SELECT 1 FROM pg_database WHERE datname = '{}'",
                    shell_single_quote(database)
                ),
            ],
        )
        .await?;
    Ok(output.trim() == "1")
}

async fn create_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
    owner: Option<&str>,
) -> crate::Result<()> {
    let mut command = vec![
        "createdb".to_owned(),
        "-U".to_owned(),
        credentials.username.clone(),
    ];
    if let Some(owner) = owner {
        command.push("-O".to_owned());
        command.push(owner.to_owned());
    }
    command.push(database.to_owned());
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            command,
        )
        .await
        .map(|_| ())
}

async fn ensure_role(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    role: &str,
    password: &str,
) -> crate::Result<()> {
    run_psql(
        provider,
        credentials,
        &format!(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{}') THEN CREATE ROLE \"{}\" LOGIN PASSWORD '{}'; ELSE ALTER ROLE \"{}\" WITH LOGIN PASSWORD '{}'; END IF; END $$;",
            shell_single_quote(role),
            sql_identifier(role),
            shell_single_quote(password),
            sql_identifier(role),
            shell_single_quote(password)
        ),
        "could not ensure postgres object role",
    )
    .await
}

async fn grant_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
    role: &str,
) -> crate::Result<()> {
    run_psql(
        provider,
        credentials,
        &format!(
            "GRANT ALL PRIVILEGES ON DATABASE \"{}\" TO \"{}\";",
            sql_identifier(database),
            sql_identifier(role)
        ),
        "could not grant postgres database privileges",
    )
    .await
}

async fn run_psql(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    sql: &str,
    error: &str,
) -> crate::Result<()> {
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "psql".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                "-d".to_owned(),
                "postgres".to_owned(),
                "-c".to_owned(),
                sql.to_owned(),
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error_value| {
            GumgumError::structured_kind(
                Subsystem::Setup,
                ErrorCode::Io,
                ErrorKind::PostgresDatabaseCreateFailed,
            )
            .likely_cause(format!("{error}; {}", error_value.to_report().message))
            .build()
        })
}

async fn drop_database(
    provider: &ProviderSpec,
    credentials: &ProviderCredentials,
    database: &str,
) -> crate::Result<()> {
    DockerEngine::local()?
        .exec_success(
            &provider.container,
            vec![("PGPASSWORD".to_owned(), credentials.password.clone())],
            vec![
                "dropdb".to_owned(),
                "--if-exists".to_owned(),
                "-U".to_owned(),
                credentials.username.clone(),
                database.to_owned(),
            ],
        )
        .await
        .map(|_| ())
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn sql_identifier(value: &str) -> String {
    value.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_readiness_requires_stable_successes() {
        let successes = [true, false, true, true]
            .into_iter()
            .scan(0, |consecutive, success| {
                if success {
                    *consecutive += 1;
                } else {
                    *consecutive = 0;
                }
                Some(*consecutive)
            })
            .collect::<Vec<_>>();

        assert_eq!(successes, vec![1, 0, 1, 2]);
    }

    #[test]
    fn postgres_readiness_error_is_provider_specific() {
        let error = GumgumError::structured_kind(
            Subsystem::Setup,
            ErrorCode::Io,
            ErrorKind::PostgresProviderReadinessFailed,
        )
        .likely_cause("connection refused")
        .build()
        .to_report();

        assert_eq!(error.kind, Some(ErrorKind::PostgresProviderReadinessFailed));
        assert_eq!(error.likely_cause.as_deref(), Some("connection refused"));
    }

    #[test]
    fn postgres_object_actions_include_database_and_dns() {
        let actions = actions("visit-counter", "visits.db.leostera.dev");
        assert!(matches!(
            actions.as_slice(),
            [
                crate::CoreAction::ProviderConfigured { provider, .. },
                crate::CoreAction::DatabaseCreated { database },
                crate::CoreAction::DnsPublished { dns, provider: dns_provider },
            ] if provider == "postgres.main"
                && database == "visit-counter"
                && dns == "visits.db.leostera.dev"
                && dns_provider == "postgres.main"
        ));
    }

    #[test]
    fn postgres_delete_for_absent_database_is_actionable() {
        let plan = crate::providers::object_provider_plan(
            Capability::Db,
            "visits",
            "visits.db.leostera.dev",
        );

        assert_eq!(plan.provider.provider, "postgres.main");
    }

    #[test]
    fn postgres_sql_literal_escapes_single_quotes() {
        assert_eq!(shell_single_quote("team's-db"), "team''s-db");
    }

    #[test]
    fn postgres_sql_identifier_escapes_double_quotes() {
        assert_eq!(sql_identifier("team\"db"), "team\"\"db");
    }
}
