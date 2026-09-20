use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

/// Connection wiring only; migrations and authoritative catalog entities follow in SCT-8.
#[derive(Clone)]
pub struct Catalog {
    pub(crate) pool: PgPool,
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

    /// Forward-only embedded migrations; SQLx rejects modified or unknown migrations.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        let migrations = vec![
            sqlx::migrate::Migration::new(
                1,
                "foundation".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0001_foundation.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                2,
                "transfer".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0002_transfer.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                3,
                "admission".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0003_admission.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                4,
                "publication".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0004_publication.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                5,
                "trees".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0005_trees.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                6,
                "draft_names".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0006_draft_names.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                7,
                "graph_work".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0007_graph_work.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                8,
                "tree_build".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0008_tree_build.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                9,
                "policy_previews".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0009_policy_previews.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                10,
                "lifecycle_jobs".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0010_lifecycle_jobs.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                11,
                "move_previews".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0011_move_previews.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                12,
                "purge".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0012_purge.sql").into(),
                false,
            ),
            sqlx::migrate::Migration::new(
                13,
                "management".into(),
                sqlx::migrate::MigrationType::Simple,
                include_str!("../migrations/0013_management.sql").into(),
                false,
            ),
        ];
        sqlx::migrate::Migrator {
            migrations: migrations.into(),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        }
        .run(&self.pool)
        .await
    }

    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
