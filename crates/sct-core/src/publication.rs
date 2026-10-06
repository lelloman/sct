use crate::{
    admission::graphs,
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Result, id, require},
    operations::check_fence,
    paging,
    roots::json,
    transfer_io::TransferEngine,
    upload::UploadRequest,
};
use sct_protocol::*;
use simple_server::database::postgres as db;

impl TransferEngine {
    pub async fn commit_upload(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<CommitReceipt> {
        let s = self.session(actor, root, upload).await?;
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, &s.root, Capability::Write).await?;
        let row = db::query(
            "SELECT state,receipt,manifest FROM transfer_sessions WHERE id=$1 AND principal_id=$2",
        )
        .bind(&s.id)
        .bind(&actor.principal)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(receipt) = row.get::<Option<serde_json::Value>, _>("receipt") {
            return Ok(serde_json::from_value(receipt)?);
        }
        require(
            row.get::<String, _>("state") == "ready" && s.admission == AdmissionState::Admitted,
            ErrorCode::InvalidRequest,
        )?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        s.check_target(&mut tx, actor).await?;
        let missing:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM session_objects p JOIN root_backends rb ON rb.root_id=p.root_id WHERE p.session_id=$1 AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state='available'))").bind(&s.id).fetch_one(&mut *tx).await?;
        require(!missing, ErrorCode::IntegrityFailure)?;
        let sql = format!(
            "{} SELECT COALESCE(sum(o.length),0)::bigint FROM projected p JOIN content_objects o ON o.root_id=$1 AND o.digest=p.digest WHERE o.kind='chunk'",
            graphs()
        );
        let unique: i64 = db::query_scalar(&sql)
            .bind(&s.root)
            .bind(&s.id)
            .fetch_one(&mut *tx)
            .await?;
        let quota: i64 = db::query_scalar("SELECT quota FROM roots WHERE id=$1")
            .bind(&s.root)
            .fetch_one(&mut *tx)
            .await?;
        require(unique <= quota, ErrorCode::QuotaExhausted)?;
        let outgoing_sql = format!(
            "{} SELECT c.generation FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.root_id=$1 AND a.root_id=$1 AND c.state='available' AND NOT EXISTS(SELECT 1 FROM projected p WHERE p.digest=c.digest)",
            graphs()
        );
        // Persist the set in a transaction-local SQL table. Object count never
        // determines application memory or the number of copied graph edges.
        db::query("CREATE TEMP TABLE outgoing_copies(generation text PRIMARY KEY) ON COMMIT DROP")
            .execute(&mut *tx)
            .await?;
        db::query(&format!("INSERT INTO outgoing_copies {outgoing_sql}"))
            .bind(&s.root)
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        let pools=db::query("SELECT a.id retained,s.id staging,a.pool_id FROM allocations a JOIN allocations s ON s.pool_id=a.pool_id AND s.root_id IS NULL WHERE a.root_id=$1 ORDER BY a.pool_id").bind(&s.root).fetch_all(&mut *tx).await?;
        for p in pools {
            let retained: String = p.get("retained");
            let staging: String = p.get("staging");
            db::query("SELECT id FROM pools WHERE id=$1 FOR UPDATE")
                .bind(p.get::<String, _>("pool_id"))
                .fetch_one(&mut *tx)
                .await?;
            let incoming:i64=db::query_scalar("SELECT COALESCE(sum(bytes),0)::bigint FROM object_copies WHERE operation_id=$1 AND allocation_id=$2 AND state='available'").bind(&s.fence.id).bind(&staging).fetch_one(&mut *tx).await?;
            let outgoing:i64=db::query_scalar("SELECT COALESCE(sum(c.bytes),0)::bigint FROM object_copies c JOIN outgoing_copies o USING(generation) WHERE c.allocation_id=$1").bind(&retained).fetch_one(&mut *tx).await?;
            let unused:i64=db::query_scalar("SELECT COALESCE(sum(bytes),0)::bigint FROM reservations WHERE operation_id=$1 AND allocation_id=$2").bind(&s.fence.id).bind(&staging).fetch_one(&mut *tx).await?;
            let n=db::query("UPDATE allocations SET occupied=occupied+$2-$3 WHERE id=$1 AND occupied+$2-$3+reserved<=capacity AND occupied >= $3").bind(&retained).bind(incoming).bind(outgoing).execute(&mut *tx).await?.rows_affected();
            require(n == 1, ErrorCode::DestinationCapacity)?;
            let n=db::query("UPDATE allocations SET reserved=reserved-$4,occupied=occupied-$2+$3,pending_cleanup=pending_cleanup+$3 WHERE id=$1 AND occupied-$2+$3+reserved-$4<=capacity AND reserved>=$4 AND occupied-pending_cleanup>=$2").bind(&staging).bind(incoming).bind(outgoing).bind(unused).execute(&mut *tx).await?.rows_affected();
            require(n == 1, ErrorCode::StagingExhausted)?;
            db::query("UPDATE object_copies SET allocation_id=$3 WHERE operation_id=$1 AND allocation_id=$2 AND state='available'").bind(&s.fence.id).bind(&staging).bind(&retained).execute(&mut *tx).await?;
            db::query("UPDATE object_copies SET allocation_id=$2,operation_id=$3,pending_charge=true WHERE generation IN (SELECT generation FROM outgoing_copies) AND allocation_id=$1").bind(&retained).bind(&staging).bind(&s.fence.id).execute(&mut *tx).await?;
        }
        db::query("DELETE FROM reservations WHERE operation_id=$1")
            .bind(&s.fence.id)
            .execute(&mut *tx)
            .await?;
        let entry = match &s.request {
            UploadRequest::Tree(request) => request.base.directory_id.as_str().to_owned(),
            UploadRequest::File(CreateUploadRequest {
                target:
                    FileTarget::Create {
                        parent_id, name, ..
                    },
                ..
            }) => {
                let entry = id();
                db::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) VALUES($1,$2,$3,$4,'file',$5)").bind(&entry).bind(&s.root).bind(parent_id.as_str()).bind(name.as_str()).bind(id()).execute(&mut *tx).await?;
                db::query("UPDATE entries SET revision=$2 WHERE id=$1")
                    .bind(parent_id.as_str())
                    .bind(id())
                    .execute(&mut *tx)
                    .await?;
                entry
            }
            UploadRequest::File(CreateUploadRequest {
                target: FileTarget::Replace { entry_id, .. },
                ..
            }) => entry_id.as_str().to_owned(),
        };
        let version = id();
        let manifest: String = row.get("manifest");
        let kind = match &s.request {
            UploadRequest::File(_) => "file",
            UploadRequest::Tree(_) => "directory",
        };
        let sequence:i64=db::query_scalar("INSERT INTO file_versions(id,root_id,entry_id,manifest,length,kind) SELECT $1,$2,$3,digest,length,$5 FROM content_objects WHERE root_id=$2 AND digest=$4 AND kind=$5 RETURNING sequence").bind(&version).bind(&s.root).bind(&entry).bind(&manifest).bind(kind).fetch_one(&mut *tx).await?;
        db::query("UPDATE entries SET current_version_id=$2,revision=$3 WHERE id=$1")
            .bind(&entry)
            .bind(&version)
            .bind(id())
            .execute(&mut *tx)
            .await?;
        db::query("UPDATE file_versions SET retired=true WHERE id IN (SELECT version_id FROM transfer_retire WHERE session_id=$1)").bind(&s.id).execute(&mut *tx).await?;
        db::query("UPDATE transfer_retire SET applied=true WHERE session_id=$1")
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        let receipt = CommitReceipt {
            version_id: version.parse()?,
            manifest_id: manifest.parse()?,
            commit_sequence: (sequence as u64).try_into()?,
        };
        db::query("UPDATE transfer_sessions SET state='committed',receipt=$2 WHERE id=$1")
            .bind(&s.id)
            .bind(json(&receipt)?)
            .execute(&mut *tx)
            .await?;
        db::query("UPDATE roots SET populated=true,revision=$2,last_commit_at=clock_timestamp() WHERE id=$1")
            .bind(&s.root)
            .bind(id())
            .execute(&mut *tx)
            .await?;
        db::query("UPDATE operations SET phase='published',state='cleaning_up' WHERE id=$1")
            .bind(&s.fence.id)
            .execute(&mut *tx)
            .await?;
        db::query("INSERT INTO transfer_audit(root_id,operation_id,event,details) VALUES($1,$2,'publication',$3)").bind(&s.root).bind(&s.fence.id).bind(json(&receipt)?).execute(&mut *tx).await?;
        tx.commit().await?;
        // Publication is already durable; cleanup failure must not change the
        // returned commit result. The background worker retains root ownership.
        let _ = self.writer.finish_operation(&s.fence, false).await;
        Ok(receipt)
    }
    pub async fn file_history(
        &self,
        actor: &Actor,
        root: &RootId,
        entry: &EntryId,
        cursor: Option<&CursorToken>,
    ) -> Result<VersionPage> {
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let revision: String = db::query_scalar("SELECT revision FROM roots WHERE id=$1 FOR SHARE")
            .bind(root.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(
            !db::query_scalar::<_, bool>("SELECT entry_purged($1)")
                .bind(entry.as_str())
                .fetch_one(&mut *tx)
                .await?,
            ErrorCode::NotFound,
        )?;
        let binding = serde_json::json!({"root":root,"entry":entry,"revision":revision});
        let before: i64 = if let Some(cursor) = cursor {
            paging::consume(&mut tx, actor, "file_history", cursor, &binding).await?
        } else {
            i64::MAX
        };
        let rows=db::query("SELECT * FROM file_versions WHERE root_id=$1 AND entry_id=$2 AND NOT retired AND sequence<$3 ORDER BY sequence DESC LIMIT 1001").bind(root.as_str()).bind(entry.as_str()).bind(before).fetch_all(&mut *tx).await?;
        let versions: Vec<FileVersion> = rows
            .iter()
            .take(1000)
            .map(|r| {
                Ok(FileVersion {
                    id: r.get::<String, _>("id").parse()?,
                    entry_id: entry.clone(),
                    manifest_id: r.get::<String, _>("manifest").parse()?,
                    length: (r.get::<i64, _>("length") as u64).try_into()?,
                    commit_sequence: (r.get::<i64, _>("sequence") as u64).try_into()?,
                })
            })
            .collect::<Result<_>>()?;
        let next_cursor = if rows.len() > 1000 {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    "file_history",
                    binding,
                    serde_json::json!(versions.last().unwrap().commit_sequence.get()),
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(VersionPage {
            versions,
            next_cursor,
        })
    }
}
