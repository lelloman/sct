//! Restored service catalogs retain history, never another host's live work.
use crate::error::{Result, id};
use db::PgConnection;
use simple_server::database::postgres as db;

pub(crate) async fn reset_runtime(db: &mut PgConnection) -> Result<()> {
    // A checkpoint can predate later physical burns. Its saved high-water mark
    // cannot authorize another append, even after all known discs are scanned.
    db::query("UPDATE archive_collections SET reconciliation_required=true")
        .execute(&mut *db)
        .await?;
    // Fence work before the enclosing restore releases graph pins. A restored
    // catalog has no verified staging/cache files and never opens old host paths.
    db::query("UPDATE archive_sets SET state='failed',failure='archive_reconciliation_required',revision=$1 WHERE state IN ('capturing','planning','awaiting_confirmation','building')").bind(id()).execute(&mut *db).await?;
    db::query("UPDATE archive_jobs SET worker_epoch=worker_epoch+1,server_epoch=0,worker_expires_at=NULL,next_attempt_at=NULL").execute(&mut *db).await?;
    db::query("UPDATE archive_exports SET finished=true,reserved_bytes=0,worker_epoch=worker_epoch+1,server_epoch=0,worker_expires_at=NULL").execute(&mut *db).await?;
    db::query("UPDATE archive_builds SET relative_path='',worker_epoch=worker_epoch+1,server_epoch=0,stage=CASE WHEN stage='available' THEN stage ELSE 'failed' END").execute(&mut *db).await?;
    db::query("DELETE FROM archive_downloads")
        .execute(&mut *db)
        .await?;
    db::query("UPDATE archive_artifacts SET state=CASE WHEN state='available' THEN 'expired' WHEN state='building' THEN 'failed' ELSE state END,relative_path=NULL,cleanup_pending=false").execute(&mut *db).await?;
    db::query("UPDATE archive_sets SET pinned_bytes=0")
        .execute(&mut *db)
        .await?;
    db::query("UPDATE archive_accounts SET cache_bytes=0,reserved_bytes=0,pinned_bytes=0")
        .execute(&mut *db)
        .await?;
    db::query("DELETE FROM archive_storage")
        .execute(&mut *db)
        .await?;
    // Copy/report/event records, immutable images' hashes/lengths, completed
    // recovery points and all reserved disc identities deliberately survive.
    Ok(())
}
