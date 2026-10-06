use simple_server::database::postgres as db;
use std::time::Duration;

use db::{PgPool, PgPoolOptions};
use simple_server::database::migrations::{
    CompletionState, MigrationEntry, MigrationObservation, MigrationPlan, MigrationVersion,
    RecordedMigration,
};

/// Connection wiring only; migrations and authoritative catalog entities follow in SCT-8.
#[derive(Clone)]
pub struct Catalog {
    pub(crate) pool: PgPool,
}

impl Catalog {
    /// Parse connection settings without requiring a running database at build time.
    pub fn new(database_url: &str) -> Result<Self, db::Error> {
        Ok(Self {
            pool: PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(3))
                .connect_lazy(database_url)?,
        })
    }

    /// Forward-only embedded migrations; SQLx rejects modified or unknown migrations.
    pub async fn migrate(&self) -> Result<(), db::migrate::MigrateError> {
        let migrations = vec![
            db::migrate::Migration::new(
                1,
                "foundation".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0001_foundation.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                2,
                "transfer".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0002_transfer.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                3,
                "admission".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0003_admission.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                4,
                "publication".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0004_publication.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                5,
                "trees".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0005_trees.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                6,
                "draft_names".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0006_draft_names.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                7,
                "graph_work".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0007_graph_work.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                8,
                "tree_build".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0008_tree_build.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                9,
                "policy_previews".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0009_policy_previews.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                10,
                "lifecycle_jobs".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0010_lifecycle_jobs.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                11,
                "move_previews".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0011_move_previews.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                12,
                "purge".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0012_purge.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                13,
                "management".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0013_management.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                14,
                "maintenance_execution".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0014_maintenance_execution.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                15,
                "mirror_repair".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0015_mirror_repair.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                16,
                "checkpoint_fencing".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0016_checkpoint_fencing.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                17,
                "checkpoint_exports".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0017_checkpoint_exports.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                18,
                "commit_freshness".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0018_commit_freshness.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                19,
                "allocation_counter_locking".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0019_allocation_counter_locking.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                20,
                "disc_archives".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0020_disc_archives.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                21,
                "archive_capture".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0021_archive_capture.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                22,
                "archive_planning".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0022_archive_planning.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                23,
                "archive_exports".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0023_archive_exports.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                24,
                "archive_downloads".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0024_archive_downloads.sql").into(),
                false,
            ),
            db::migrate::Migration::new(
                25,
                "archive_copies".into(),
                db::migrate::MigrationType::Simple,
                include_str!("../migrations/0025_archive_copies.sql").into(),
                false,
            ),
        ];
        let migrator = db::migrate::Migrator {
            migrations: migrations.into(),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
        let namespace = "sct/postgres-catalog".to_owned();
        let plan = MigrationPlan {
            namespace: namespace.clone(),
            entries: migrator
                .iter()
                .filter(|migration| migration.migration_type.is_up_migration())
                .map(|migration| MigrationEntry {
                    version: MigrationVersion::Number(migration.version as u64),
                    name: migration.description.to_string(),
                    digest: Some(hex::encode(&migration.checksum)),
                })
                .collect(),
        };
        let has_ledger: Option<String> =
            db::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
                .fetch_one(&self.pool)
                .await?;
        let rows = if has_ledger.is_none() {
            Vec::new()
        } else {
            db::query("SELECT version, description, success, encode(checksum, 'hex') AS checksum FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&self.pool)
                .await?
        };
        let observation = MigrationObservation {
            namespace,
            entries: rows
                .iter()
                .map(|row| {
                    let version: i64 = row.get("version");
                    Ok(RecordedMigration {
                        entry: MigrationEntry {
                            version: MigrationVersion::Number(u64::try_from(version).map_err(
                                |_| {
                                    db::migrate::MigrateError::Source(Box::new(
                                        std::io::Error::new(
                                            std::io::ErrorKind::InvalidData,
                                            format!("invalid migration version {version}"),
                                        ),
                                    ))
                                },
                            )?),
                            name: row.get("description"),
                            digest: Some(row.get("checksum")),
                        },
                        state: if row.get("success") {
                            CompletionState::Applied
                        } else {
                            CompletionState::Dirty
                        },
                    })
                })
                .collect::<Result<Vec<_>, db::migrate::MigrateError>>()?,
            high_water_mark: None,
        };
        plan.inspect(&observation)
            .map_err(|error| db::migrate::MigrateError::Source(Box::new(error)))?;
        migrator.run(&self.pool).await
    }

    pub async fn ping(&self) -> Result<(), db::Error> {
        db::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
