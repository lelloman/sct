//! Consistent catalog artifacts. This transaction never locks the writer lease.
use crate::{
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    roots::json,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use simple_server::database::postgres as db;
use simple_server::io::AsyncWriteExt;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogHeader {
    pub format: u32,
    pub schema_version: i64,
    pub schema_digest: String,
    pub checkpoint_id: OperationId,
    pub generation_watermark: ByteCount,
    pub tables: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRecord {
    pub table: String,
    pub row: serde_json::Value,
}
fn io(_: std::io::Error) -> Error {
    Error(ErrorCode::DestinationUnavailable)
}
pub(crate) fn identifier(value: &str) -> Result<String> {
    require(
        !value.is_empty()
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'),
        ErrorCode::InvalidRequest,
    )?;
    Ok(format!("\"{value}\""))
}
struct Artifact {
    file: simple_server::fs::File,
    hash: Sha256,
}
impl Artifact {
    async fn create(path: &Path) -> Result<Self> {
        let file = simple_server::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
            .map_err(io)?;
        Ok(Self {
            file,
            hash: Sha256::new(),
        })
    }
    async fn line<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        self.hash.update(&bytes);
        self.file.write_all(&bytes).await.map_err(io)
    }
    async fn finish(mut self) -> Result<ChunkDigest> {
        self.file.flush().await.map_err(io)?;
        self.file.sync_all().await.map_err(io)?;
        Ok(format!("{:x}", self.hash.finalize()).parse()?)
    }
}
impl TransferEngine {
    pub fn with_checkpoint_directory(mut self, path: PathBuf) -> Self {
        self.checkpoint_directory = Some(path);
        self
    }
    pub fn checkpoint_directory(&self) -> Option<&Path> {
        self.checkpoint_directory.as_deref()
    }
    pub(crate) async fn export_ready_checkpoint(&self) -> Result<()> {
        let Some(directory) = &self.checkpoint_directory else {
            return Ok(());
        };
        let Some(_export) = self.checkpoint_export.try_lock() else {
            return Ok(());
        };
        let checkpoint: Option<String> = {
            let mut tx = self.writer.control_transaction().await?;
            db::query_scalar("SELECT c.id FROM checkpoints c JOIN checkpoint_barrier b ON b.checkpoint_id=c.id WHERE c.state='exporting' AND b.phase='capturing' ORDER BY c.created_at LIMIT 1").fetch_optional(&mut *tx).await?
        };
        let Some(checkpoint) = checkpoint else {
            return Ok(());
        };
        {
            let mut tx = self.writer.control_transaction().await?;
            db::query(
                "UPDATE checkpoints SET export_server_epoch=$2 WHERE id=$1 AND state='exporting'",
            )
            .bind(&checkpoint)
            .bind(self.writer.epoch)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        let result = self.export_checkpoint(&checkpoint, directory).await;
        if let Err(error) = &result {
            let mut tx = self.writer.control_transaction().await?;
            db::query("UPDATE checkpoints SET state='failed',failure=$2,revision=$3 WHERE id=$1 AND state='exporting' AND export_server_epoch=$4").bind(&checkpoint).bind(json(&error.0)?.as_str().unwrap()).bind(id()).bind(self.writer.epoch).execute(&mut *tx).await?;
            db::query("DELETE FROM checkpoint_barrier WHERE checkpoint_id=$1 AND server_epoch=$2")
                .bind(&checkpoint)
                .bind(self.writer.epoch)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        result
    }
    async fn export_checkpoint(&self, checkpoint: &str, directory: &Path) -> Result<()> {
        // Establish the MVCC snapshot before releasing the capture barrier.
        let mut snapshot = self.writer.catalog.pool.begin().await?;
        db::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        db::query("SET LOCAL TIME ZONE 'UTC'")
            .execute(&mut *snapshot)
            .await?;
        let row=db::query("SELECT c.generation_watermark,c.principal_id,c.credential_id,b.server_epoch FROM checkpoints c JOIN checkpoint_barrier b ON b.checkpoint_id=c.id WHERE c.id=$1 AND c.state='exporting' AND b.phase='capturing'").bind(checkpoint).fetch_optional(&mut *snapshot).await?.ok_or(Error(ErrorCode::RevisionConflict))?;
        require(
            row.get::<i64, _>("server_epoch") == self.writer.epoch,
            ErrorCode::RevisionConflict,
        )?;
        let actor = Actor {
            principal: row.get("principal_id"),
            credential: row
                .get::<Option<String>, _>("credential_id")
                .unwrap_or_default(),
        };
        let watermark: ByteCount = (row.get::<i64, _>("generation_watermark") as u64).try_into()?;
        {
            let mut tx = self.writer.control_transaction().await?;
            auth::admin(&mut tx, &actor).await?;
            let changed = db::query(
                "UPDATE checkpoints SET export_server_epoch=$2 WHERE id=$1 AND state='exporting'",
            )
            .bind(checkpoint)
            .bind(self.writer.epoch)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            require(changed == 1, ErrorCode::RevisionConflict)?;
            db::query("DELETE FROM checkpoint_barrier WHERE checkpoint_id=$1 AND server_epoch=$2")
                .bind(checkpoint)
                .bind(self.writer.epoch)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        simple_server::fs::create_dir_all(directory)
            .await
            .map_err(io)?;
        let destination = directory.join(checkpoint);
        simple_server::fs::create_dir(&destination)
            .await
            .map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            simple_server::fs::set_permissions(
                &destination,
                std::fs::Permissions::from_mode(0o700),
            )
            .await
            .map_err(io)?;
        }
        let schema: String = db::query_scalar("SELECT current_schema()")
            .fetch_one(&mut *snapshot)
            .await?;
        let tables:Vec<String>=db::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname=current_schema() AND tablename<>'_sqlx_migrations' ORDER BY tablename").fetch_all(&mut *snapshot).await?;
        let migrations=db::query("SELECT version,encode(checksum,'hex') checksum FROM _sqlx_migrations WHERE success ORDER BY version").fetch_all(&mut *snapshot).await?;
        let schema_version = migrations
            .last()
            .ok_or(Error(ErrorCode::IntegrityFailure))?
            .get("version");
        let mut schema_hash = Sha256::new();
        for migration in migrations {
            schema_hash.update(migration.get::<i64, _>("version").to_be_bytes());
            schema_hash.update(migration.get::<String, _>("checksum").as_bytes());
        }
        let mut catalog = Artifact::create(&destination.join("catalog.jsonl")).await?;
        catalog
            .line(&CatalogHeader {
                format: 1,
                schema_version,
                schema_digest: format!("{:x}", schema_hash.finalize()),
                checkpoint_id: checkpoint.parse()?,
                generation_watermark: watermark,
                tables: tables.clone(),
            })
            .await?;
        for table in tables {
            let query = format!(
                "DECLARE sct_export_cursor NO SCROLL CURSOR FOR SELECT to_jsonb(t) AS row FROM {}.{} t",
                identifier(&schema)?,
                identifier(&table)?
            );
            db::query(&query).execute(&mut *snapshot).await?;
            loop {
                let rows = db::query("FETCH FORWARD 128 FROM sct_export_cursor")
                    .fetch_all(&mut *snapshot)
                    .await?;
                let done = rows.len() < 128;
                for row in rows {
                    catalog
                        .line(&CatalogRecord {
                            table: table.clone(),
                            row: row.get("row"),
                        })
                        .await?;
                }
                if done {
                    break;
                }
            }
            db::query("CLOSE sct_export_cursor")
                .execute(&mut *snapshot)
                .await?;
        }
        let catalog_digest = catalog.finish().await?;
        let mut inventory = Artifact::create(&destination.join("inventory.jsonl")).await?;
        let mut after = 0i64;
        loop {
            let rows=db::query("SELECT c.* FROM checkpoint_objects p JOIN object_copies c ON c.generation=p.generation WHERE p.checkpoint_id=$1 AND c.created_sequence>$2 ORDER BY c.created_sequence LIMIT 128").bind(checkpoint).bind(after).fetch_all(&mut *snapshot).await?;
            let done = rows.len() < 128;
            for row in rows {
                after = row.get("created_sequence");
                inventory
                    .line(&CheckpointObject {
                        root_id: row.get::<String, _>("root_id").parse()?,
                        backend_id: row.get::<String, _>("backend_id").parse()?,
                        digest: row.get::<String, _>("digest").parse()?,
                        generation: row.get("generation"),
                        bytes: (row.get::<i64, _>("bytes") as u64).try_into()?,
                    })
                    .await?;
            }
            if done {
                break;
            }
        }
        let inventory_digest = inventory.finish().await?;
        snapshot.commit().await?;
        let descriptor = CheckpointDescriptor {
            format: 1,
            checkpoint_id: checkpoint.parse()?,
            generation_watermark: watermark,
            catalog_digest,
            inventory_digest,
            package_state: RecoveryPackageState::CatalogOnly,
            storage_format_version: 1,
        };
        let mut artifact = Artifact::create(&destination.join("descriptor.json")).await?;
        artifact.line(&descriptor).await?;
        artifact.finish().await?;
        simple_server::fs::File::open(&destination)
            .await
            .map_err(io)?
            .sync_all()
            .await
            .map_err(io)?;
        simple_server::fs::File::open(directory)
            .await
            .map_err(io)?
            .sync_all()
            .await
            .map_err(io)?;
        let mut tx = self.writer.control_transaction().await?;
        auth::admin(&mut tx, &actor).await?;
        let changed=db::query("UPDATE checkpoints SET state='exported',descriptor=$2,revision=$3 WHERE id=$1 AND state='exporting' AND export_server_epoch=$4").bind(checkpoint).bind(json(&descriptor)?).bind(id()).bind(self.writer.epoch).execute(&mut *tx).await?.rows_affected();
        require(changed == 1, ErrorCode::RevisionConflict)?;
        tx.commit().await?;
        Ok(())
    }
}

impl Writer {
    pub async fn checkpoint_descriptor(
        &self,
        actor: &Actor,
        checkpoint: &OperationId,
    ) -> Result<CheckpointDescriptor> {
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = db::query("SELECT state,descriptor FROM checkpoints WHERE id=$1")
            .bind(checkpoint.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        require(
            row.get::<String, _>("state") == "exported",
            ErrorCode::RevisionConflict,
        )?;
        Ok(serde_json::from_value(
            row.get::<Option<serde_json::Value>, _>("descriptor")
                .ok_or(Error(ErrorCode::IntegrityFailure))?,
        )?)
    }
    pub async fn checkpoint_inventory(
        &self,
        actor: &Actor,
        checkpoint: &OperationId,
        request: &ListRequest,
    ) -> Result<CheckpointInventoryPage> {
        let mut tx = self.control_transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let exists: bool = db::query_scalar("SELECT EXISTS(SELECT 1 FROM checkpoints WHERE id=$1)")
            .bind(checkpoint.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(exists, ErrorCode::NotFound)?;
        let scope = format!("checkpoint-inventory:{}", checkpoint.as_str());
        let after: i64 = match &request.cursor {
            Some(c) => {
                crate::paging::consume(&mut tx, actor, &scope, c, &json(&request.limit)?).await?
            }
            None => 0,
        };
        let rows=db::query("SELECT c.* FROM checkpoint_objects p JOIN object_copies c ON c.generation=p.generation WHERE p.checkpoint_id=$1 AND c.created_sequence>$2 ORDER BY c.created_sequence LIMIT $3").bind(checkpoint.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let limit = request.limit.get() as usize;
        let next_cursor = if rows.len() > limit {
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    json(&request.limit)?,
                    json(&rows[limit - 1].get::<i64, _>("created_sequence"))?,
                )
                .await?,
            )
        } else {
            None
        };
        let objects = rows
            .iter()
            .take(limit)
            .map(|row| -> Result<CheckpointObject> {
                Ok(CheckpointObject {
                    root_id: row.get::<String, _>("root_id").parse()?,
                    backend_id: row.get::<String, _>("backend_id").parse()?,
                    digest: row.get::<String, _>("digest").parse()?,
                    generation: row.get("generation"),
                    bytes: (row.get::<i64, _>("bytes") as u64).try_into()?,
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(CheckpointInventoryPage {
            objects,
            next_cursor,
        })
    }
}
impl TransferEngine {
    pub async fn checkpoint_artifact(
        &self,
        actor: &Actor,
        checkpoint: &OperationId,
        artifact: &str,
    ) -> Result<simple_server::fs::File> {
        let descriptor = self.writer.checkpoint_descriptor(actor, checkpoint).await?;
        require(
            matches!(
                artifact,
                "catalog.jsonl" | "inventory.jsonl" | "descriptor.json"
            ),
            ErrorCode::NotFound,
        )?;
        // Use the canonical catalog identifier; user input is never a filesystem path.
        let directory = self
            .checkpoint_directory
            .as_ref()
            .ok_or(Error(ErrorCode::UnsupportedOperation))?;
        simple_server::fs::File::open(
            directory
                .join(descriptor.checkpoint_id.as_str())
                .join(artifact),
        )
        .await
        .map_err(io)
    }
}
