use crate::{ErrorCode, GumgumError, Subsystem};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{path::Path, str::FromStr};

static GRAPH_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn migrate_graph_store(path: &Path) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not create graph directory",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::InvalidArgs,
                "could not build graph database URL",
            )
            .likely_cause(source.to_string())
            .build()
        })?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|source| {
            GumgumError::structured(
                Subsystem::Config,
                ErrorCode::Io,
                "could not open graph database for migrations",
            )
            .likely_cause(source.to_string())
            .build()
        })?;
    GRAPH_MIGRATOR.run(&pool).await.map_err(|source| {
        GumgumError::structured(
            Subsystem::Config,
            ErrorCode::Io,
            "could not run graph database migrations",
        )
        .likely_cause(source.to_string())
        .build()
    })?;
    pool.close().await;
    Ok(())
}
