use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

/// Connection wiring only; migrations and authoritative catalog entities follow in SCT-8.
#[derive(Clone)]
pub struct Catalog {
    pool: PgPool,
}

impl Catalog {
    /// Parse connection settings without requiring a running database at build time.
    pub fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(3))
                .connect_lazy(database_url)?,
        })
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
