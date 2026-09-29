//! Connection settings for both offline archive SQLite files.

use crate::Result;
use simple_server::database::sqlite::{
    ConnectionObservation, ConnectionPolicy, JournalMode, Synchronous,
};
use sqlx::{ConnectOptions, Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{path::Path, time::Duration};

pub(crate) async fn connect_archive(path: &Path) -> Result<SqliteConnection> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .statement_cache_capacity(32)
        .pragma("cache_size", "-8192")
        .pragma("temp_store", "FILE")
        .disable_statement_logging();
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let policy = ConnectionPolicy::new()
        .foreign_keys(true)
        .journal_mode(JournalMode::Delete)
        .synchronous(Synchronous::Full);
    for command in policy.commands()? {
        sqlx::query(&command).execute(&mut connection).await?;
    }
    sqlx::query("SELECT 1").execute(&mut connection).await?;
    let observed = ConnectionObservation {
        foreign_keys: sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
            .fetch_one(&mut connection)
            .await?
            != 0,
        busy_timeout: Duration::from_millis(
            sqlx::query_scalar::<_, i64>("PRAGMA busy_timeout")
                .fetch_one(&mut connection)
                .await?
                .try_into()?,
        ),
        journal_mode: sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut connection)
            .await?,
        synchronous: sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
            .fetch_one(&mut connection)
            .await?
            .try_into()?,
        wal_autocheckpoint: sqlx::query_scalar::<_, i64>("PRAGMA wal_autocheckpoint")
            .fetch_one(&mut connection)
            .await?
            .try_into()?,
    };
    policy.verify(&observed)?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn archive_settings_keep_durable_journal_and_local_cache_choices() -> Result<()> {
        let path = std::env::temp_dir().join(format!("sct-07a-{}.sqlite", uuid::Uuid::new_v4()));
        std::fs::File::create(&path)?;
        let mut connection = connect_archive(&path).await?;
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut connection)
            .await?;
        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut connection)
            .await?;
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut connection)
            .await?;
        let cache_size: i64 = sqlx::query_scalar("PRAGMA cache_size")
            .fetch_one(&mut connection)
            .await?;
        let temp_store: i64 = sqlx::query_scalar("PRAGMA temp_store")
            .fetch_one(&mut connection)
            .await?;
        assert_eq!(
            (foreign_keys, journal.as_str(), synchronous),
            (1, "delete", 2)
        );
        assert_eq!((cache_size, temp_store), (-8192, 1));
        connection.close().await?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}
