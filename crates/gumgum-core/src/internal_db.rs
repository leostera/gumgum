use crate::{ErrorCode, ErrorKind, GumgumError, Subsystem};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{path::Path, str::FromStr};

static GRAPH_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn migrate_graph_store(path: &Path) -> crate::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|source| {
            config_error(ErrorCode::Io, ErrorKind::GraphDirectoryCreateFailed, source)
        })?;
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|source| {
            config_error(
                ErrorCode::InvalidArgs,
                ErrorKind::GraphDatabaseUrlBuildFailed,
                source,
            )
        })?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|source| {
            config_error(ErrorCode::Io, ErrorKind::GraphDatabaseOpenFailed, source)
        })?;
    GRAPH_MIGRATOR.run(&pool).await.map_err(|source| {
        config_error(
            ErrorCode::Io,
            ErrorKind::GraphDatabaseMigrationFailed,
            source,
        )
    })?;
    pool.close().await;
    Ok(())
}

fn config_error(code: ErrorCode, kind: ErrorKind, source: impl std::fmt::Display) -> GumgumError {
    GumgumError::structured_kind(Subsystem::Config, code, kind)
        .likely_cause(source.to_string())
        .build()
}
