use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, require},
    operations::check_fence,
    paging,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use simple_server::database::postgres as db;

impl TransferEngine {
    pub async fn payload_permit(&self) -> Result<simple_server::sync::SemaphoreGuardArc> {
        Ok(self.ingress.acquire_arc().await)
    }
    /// Called before HTTP body consumption and again after receiving the body.
    pub async fn chunk_permission(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
        chunk: &ChunkDigest,
    ) -> Result<u64> {
        let s = self.session(actor, root, upload).await?;
        require(
            matches!(s.state, SessionState::Open | SessionState::Ready)
                && s.admission == AdmissionState::Admitted,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, &s.root, Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let length:Option<i64>=db::query_scalar("SELECT o.length FROM session_objects p JOIN content_objects o USING(root_id,digest) WHERE p.session_id=$1 AND p.root_id=$2 AND p.digest=$3 AND o.kind='chunk'").bind(&s.id).bind(&s.root).bind(chunk.as_str()).fetch_optional(&mut *tx).await?;
        Ok(length.ok_or(Error(ErrorCode::InvalidRequest))? as u64)
    }
    pub async fn missing_chunks(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
        cursor: Option<&CursorToken>,
    ) -> Result<MissingChunkPage> {
        let s = self.session(actor, root, upload).await?;
        require(
            s.admission == AdmissionState::Admitted,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, &s.root, Capability::Write).await?;
        let binding = serde_json::json!({"root":root,"upload":upload});
        let after: String = if let Some(cursor) = cursor {
            paging::consume(&mut tx, actor, "missing_chunks", cursor, &binding).await?
        } else {
            String::new()
        };
        let rows=db::query("SELECT o.digest,o.length FROM session_objects p JOIN content_objects o USING(root_id,digest) WHERE p.session_id=$1 AND p.root_id=$2 AND o.kind='chunk' AND o.digest>$3 AND EXISTS(SELECT 1 FROM root_backends rb WHERE rb.root_id=p.root_id AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state='available')) ORDER BY o.digest LIMIT 1001").bind(&s.id).bind(&s.root).bind(after).fetch_all(&mut *tx).await?;
        let chunks: Vec<ChunkReference> = rows
            .iter()
            .take(1000)
            .map(|r| {
                Ok(ChunkReference {
                    digest: r.get::<String, _>("digest").parse()?,
                    length: (r.get::<i64, _>("length") as u64).try_into()?,
                })
            })
            .collect::<Result<_>>()?;
        let next_cursor = if rows.len() > 1000 {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    "missing_chunks",
                    binding,
                    serde_json::json!(chunks.last().unwrap().digest.as_str()),
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(MissingChunkPage {
            chunks,
            next_cursor,
        })
    }
    pub async fn upload_chunk(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
        chunk: &ChunkDigest,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let length = self.chunk_permission(actor, root, upload, chunk).await?;
        require(
            bytes.len() as u64 == length && digest(&bytes) == chunk.as_str(),
            ErrorCode::IntegrityFailure,
        )?;
        let s = self.session(actor, root, upload).await?;
        self.reconcile_writes(&s.fence).await?;
        self.cleanup_copies(&s.fence).await?;
        let backends = {
            let mut tx = self.writer.transaction().await?;
            db::query_scalar::<_, String>(
                "SELECT backend_id FROM root_backends WHERE root_id=$1 ORDER BY backend_id",
            )
            .bind(&s.root)
            .fetch_all(&mut *tx)
            .await?
        };
        for backend in backends {
            self.write_copy(&s.fence, chunk, &backend.parse()?, bytes.clone())
                .await?;
        }
        Ok(())
    }
    pub async fn seal_upload(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus> {
        let s = self.session(actor, root, upload).await?;
        require(
            matches!(s.state, SessionState::Open | SessionState::Ready)
                && s.admission == AdmissionState::Admitted,
            ErrorCode::InvalidRequest,
        )?;
        self.reconcile_writes(&s.fence).await?;
        self.cleanup_copies(&s.fence).await?;
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, &s.root, Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let missing:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM session_objects p JOIN content_objects o USING(root_id,digest) JOIN root_backends rb ON rb.root_id=p.root_id WHERE p.session_id=$1 AND o.kind='chunk' AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state='available'))").bind(&s.id).fetch_one(&mut *tx).await?;
        require(!missing, ErrorCode::InvalidManifest)?;
        let rows=db::query("SELECT o.digest,o.body,rb.backend_id FROM session_objects p JOIN content_objects o USING(root_id,digest) JOIN root_backends rb ON rb.root_id=p.root_id WHERE p.session_id=$1 AND o.kind<>'chunk' AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state='available') ORDER BY o.digest,rb.backend_id LIMIT 8").bind(&s.id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        for row in rows {
            self.write_copy(
                &s.fence,
                &row.get::<String, _>("digest").parse()?,
                &row.get::<String, _>("backend_id").parse()?,
                row.get("body"),
            )
            .await?;
        }
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &s.root).await?;
        authorize(&mut tx, actor, &s.root, Capability::Write).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let missing:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM session_objects p JOIN root_backends rb ON rb.root_id=p.root_id WHERE p.session_id=$1 AND NOT EXISTS(SELECT 1 FROM object_copies c WHERE c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state='available'))").bind(&s.id).fetch_one(&mut *tx).await?;
        if !missing {
            db::query("UPDATE transfer_sessions SET state='ready' WHERE id=$1 AND state='open'")
                .bind(&s.id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.upload_status(actor, root, upload).await
    }
}
