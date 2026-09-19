use crate::{
    auth::{Actor, authorize},
    content_catalog::{edge, lock_content_root, object},
    error::{Error, Result, id, require},
    operations::{OperationFence, check_fence},
    policy::effective,
    roots::{json, lock_root, revision},
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

pub(crate) struct Session {
    pub id: String,
    pub root: String,
    pub fence: OperationFence,
    pub request: CreateUploadRequest,
    pub state: SessionState,
    pub admission: AdmissionState,
    pub phase: String,
}
pub(crate) async fn check_target(
    db: &mut PgConnection,
    root: &str,
    request: &CreateUploadRequest,
) -> Result<EffectivePolicy> {
    let (entry, expected, kind) = match &request.target {
        FileTarget::Create {
            parent_id,
            expected_parent_revision,
            ..
        } => (parent_id.as_str(), expected_parent_revision, "directory"),
        FileTarget::Replace {
            entry_id,
            expected_revision,
        } => (entry_id.as_str(), expected_revision, "file"),
    };
    let row = sqlx::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2 AND NOT hidden")
        .bind(root)
        .bind(entry)
        .fetch_optional(&mut *db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    require(
        row.get::<String, _>("kind") == kind,
        ErrorCode::InvalidRequest,
    )?;
    revision(&row, expected)?;
    if let FileTarget::Create { name, .. } = &request.target {
        let length:i64=sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,name FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.name FROM entries e JOIN a ON e.id=a.parent_id) SELECT COALESCE(sum(char_length(name)+1) FILTER(WHERE parent_id IS NOT NULL),0)::bigint FROM a").bind(entry).fetch_one(&mut *db).await?;
        require(
            length + (name.as_str().chars().count() as i64) < 4096,
            ErrorCode::InvalidPath,
        )?;
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE root_id=$1 AND parent_id=$2 AND name=$3 AND NOT hidden)").bind(root).bind(entry).bind(name.as_str()).fetch_one(&mut *db).await?;
        require(!exists, ErrorCode::RevisionConflict)?;
    }
    let policy = effective(db, root, Some(entry)).await?;
    require(
        policy.versioning == VersioningMode::IndependentFiles,
        ErrorCode::UnsupportedOperation,
    )?;
    require(
        request.expected_policy_revision == policy.revision,
        ErrorCode::RevisionConflict,
    )?;
    Ok(policy)
}
impl TransferEngine {
    pub async fn create_upload(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &CreateUploadRequest,
    ) -> Result<TransferStatus> {
        request.manifest.validate()?;
        let body = canonical(&request.manifest)?;
        let manifest = digest(&body);
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        // Serialize exact retries before checking the durable root owner.
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        if let Some(row)=sqlx::query("SELECT id,request FROM transfer_sessions WHERE root_id=$1 AND principal_id=$2 AND key=$3").bind(root.as_str()).bind(&actor.principal).bind(request.idempotency_key.as_str()).fetch_optional(&mut *tx).await? {
            require(row.get::<serde_json::Value,_>("request")==json(request)?,ErrorCode::IdempotencyConflict)?;
            let upload:UploadId=row.get::<String,_>("id").parse()?;tx.commit().await?;return self.upload_status(actor,root,&upload).await;
        }
        lock_root(&mut tx, root.as_str()).await?;
        let enabled: bool = sqlx::query_scalar("SELECT state='enabled' FROM roots WHERE id=$1")
            .bind(root.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(enabled, ErrorCode::DestinationUnavailable)?;
        check_target(&mut tx, root.as_str(), request).await?;
        let operation = id();
        let upload = id();
        sqlx::query("INSERT INTO operations(id,root_id,principal_id,kind,state,phase,server_epoch,worker_expires_at) VALUES($1,$2,$3,'upload','active','planning',$4,clock_timestamp()+interval '30 seconds')").bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(self.writer.epoch).execute(&mut *tx).await?;
        object(
            &mut tx,
            root.as_str(),
            &manifest,
            "file",
            request.manifest.length.get(),
            Some(request.manifest.height),
            Some(&body),
        )
        .await?;
        if let Some(tree) = &request.manifest.tree {
            object(
                &mut tx,
                root.as_str(),
                tree.as_str(),
                "page",
                request.manifest.length.get(),
                Some(request.manifest.height),
                None,
            )
            .await?;
            edge(
                &mut tx,
                root.as_str(),
                &manifest,
                0,
                tree.as_str(),
                request.manifest.length.get(),
                request.manifest.chunks.get(),
            )
            .await?;
        }
        sqlx::query("INSERT INTO transfer_sessions(id,root_id,operation_id,principal_id,key,request,manifest,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,clock_timestamp()+make_interval(secs=>$8))").bind(&upload).bind(root.as_str()).bind(operation).bind(&actor.principal).bind(request.idempotency_key.as_str()).bind(json(request)?).bind(&manifest).bind(self.expiry_seconds as f64).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO session_objects(root_id,session_id,digest) VALUES($1,$2,$3)")
            .bind(root.as_str())
            .bind(&upload)
            .bind(manifest)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.upload_status(actor, root, &upload.parse()?).await
    }
    pub(crate) async fn session(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<Session> {
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        let row=sqlx::query("SELECT s.*,o.epoch,o.server_epoch,o.phase,(o.worker_expires_at IS NULL OR o.worker_expires_at<=clock_timestamp()) worker_expired FROM transfer_sessions s JOIN operations o ON o.id=s.operation_id WHERE s.id=$1 AND s.root_id=$2 AND s.principal_id=$3").bind(upload.as_str()).bind(root.as_str()).bind(&actor.principal).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        sqlx::query("UPDATE transfer_sessions SET state='expired' WHERE id=$1 AND state IN ('open','ready') AND expires_at<=clock_timestamp()").bind(upload.as_str()).execute(&mut *tx).await?;
        sqlx::query("UPDATE transfer_sessions SET expires_at=clock_timestamp()+make_interval(secs=>$2) WHERE id=$1 AND state IN ('open','ready') AND expires_at>clock_timestamp()").bind(upload.as_str()).bind(self.expiry_seconds as f64).execute(&mut *tx).await?;
        let state: String = sqlx::query_scalar("SELECT state FROM transfer_sessions WHERE id=$1")
            .bind(upload.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let mut fence = OperationFence {
            id: row.get("operation_id"),
            root: root.as_str().into(),
            epoch: row.get("epoch"),
        };
        tx.commit().await?;
        if (row.get::<bool, _>("worker_expired")
            || row.get::<i64, _>("server_epoch") != self.writer.epoch)
            && matches!(state.as_str(), "open" | "ready")
        {
            fence = self.writer.recover_operation(&fence.id()).await?;
        }
        Ok(Session {
            id: upload.as_str().into(),
            root: root.as_str().into(),
            fence,
            request: serde_json::from_value(row.get("request"))?,
            state: serde_json::from_value(serde_json::json!(state))?,
            admission: serde_json::from_value(serde_json::json!(
                row.get::<String, _>("admission")
            ))?,
            phase: row.get("phase"),
        })
    }
    pub async fn upload_status(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus> {
        self.session(actor, root, upload).await?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        let row=sqlx::query("SELECT *,floor(extract(epoch FROM expires_at)*1000)::bigint expires_ms FROM transfer_sessions WHERE id=$1 AND root_id=$2 AND principal_id=$3").bind(upload.as_str()).bind(root.as_str()).bind(&actor.principal).fetch_one(&mut *tx).await?;
        let mut result = status(&row)?;
        let copies=sqlx::query("SELECT rb.backend_id,COALESCE(sum(c.bytes) FILTER(WHERE c.state='available'),0)::bigint acknowledged,COALESCE(bool_and(COALESCE(c.state='available',false)),false) complete FROM root_backends rb LEFT JOIN session_objects p ON p.root_id=rb.root_id AND p.session_id=$2 LEFT JOIN object_copies c ON c.root_id=p.root_id AND c.digest=p.digest AND c.backend_id=rb.backend_id AND c.state IN ('available','writing') WHERE rb.root_id=$1 GROUP BY rb.backend_id ORDER BY rb.backend_id").bind(root.as_str()).bind(upload.as_str()).fetch_all(&mut *tx).await?;
        result.upload.copies = copies
            .iter()
            .map(|r| {
                Ok(CopyProgress {
                    backend_id: r.get::<String, _>("backend_id").parse()?,
                    acknowledged_bytes: (r.get::<i64, _>("acknowledged") as u64).try_into()?,
                    state: if r.get::<bool, _>("complete") {
                        CopyAcknowledgement::Acknowledged
                    } else {
                        CopyAcknowledgement::Pending
                    },
                })
            })
            .collect::<Result<_>>()?;
        Ok(result)
    }
    pub async fn upload_page(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
        expected: &ManifestId,
        bytes: &[u8],
    ) -> Result<()> {
        let s = self.session(actor, root, upload).await?;
        require(
            s.state == SessionState::Open && s.admission == AdmissionState::Planning,
            ErrorCode::InvalidRequest,
        )?;
        self.writer
            .store_manifest_page(&s.fence, expected, bytes)
            .await
    }
    pub async fn abort_upload(
        &self,
        actor: &Actor,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus> {
        let s = self.session(actor, root, upload).await?;
        require(
            s.state != SessionState::Committed,
            ErrorCode::RevisionConflict,
        )?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        sqlx::query("UPDATE transfer_sessions SET state='aborted' WHERE id=$1 AND state IN ('open','ready')").bind(upload.as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        if matches!(s.state, SessionState::Open | SessionState::Ready) {
            self.writer.release_unused_reservations(&s.fence).await?;
            let _ = self.writer.finish_operation(&s.fence, true).await;
        }
        self.upload_status(actor, root, upload).await
    }
    pub(crate) async fn expand_plan(&self, s: &Session) -> Result<bool> {
        let mut tx = self.writer.transaction().await?;
        lock_content_root(&mut tx, &s.root).await?;
        check_fence(&mut tx, &s.fence, self.writer.epoch).await?;
        let rows=sqlx::query("SELECT o.* FROM session_objects p JOIN content_objects o USING(root_id,digest) WHERE p.session_id=$1 AND p.root_id=$2 AND NOT p.expanded ORDER BY p.digest LIMIT 64").bind(&s.id).bind(&s.root).fetch_all(&mut *tx).await?;
        for row in rows {
            let hash: String = row.get("digest");
            let kind: String = row.get("kind");
            if kind != "chunk" {
                require(
                    row.get::<Option<Vec<u8>>, _>("body").is_some(),
                    ErrorCode::InvalidManifest,
                )?;
            }
            let valid:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM content_edges e JOIN content_objects c ON c.root_id=e.root_id AND c.digest=e.child WHERE e.root_id=$1 AND e.parent=$2 AND (e.logical_length<>c.length OR e.chunk_count<>CASE WHEN c.kind='chunk' THEN 1 ELSE (SELECT COALESCE(sum(x.chunk_count),0) FROM content_edges x WHERE x.root_id=c.root_id AND x.parent=c.digest) END))").bind(&s.root).bind(&hash).fetch_one(&mut *tx).await?;
            require(valid, ErrorCode::InvalidManifest)?;
            sqlx::query("INSERT INTO session_objects(root_id,session_id,digest) SELECT root_id,$1,child FROM content_edges WHERE root_id=$2 AND parent=$3 ON CONFLICT DO NOTHING").bind(&s.id).bind(&s.root).bind(&hash).execute(&mut *tx).await?;
            sqlx::query("UPDATE session_objects SET expanded=true WHERE root_id=$1 AND session_id=$2 AND digest=$3").bind(&s.root).bind(&s.id).bind(hash).execute(&mut *tx).await?;
        }
        let complete:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM session_objects WHERE root_id=$1 AND session_id=$2 AND NOT expanded)").bind(&s.root).bind(&s.id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(complete)
    }
}
fn status(row: &PgRow) -> Result<TransferStatus> {
    Ok(TransferStatus {
        upload: UploadInspection {
            id: row.get::<String, _>("id").parse()?,
            root_id: row.get::<String, _>("root_id").parse()?,
            operation_id: row.get::<String, _>("operation_id").parse()?,
            state: serde_json::from_value(serde_json::json!(row.get::<String, _>("state")))?,
            expires_at_ms: (row.get::<i64, _>("expires_ms") as u64).try_into()?,
            copies: vec![],
            result: row
                .get::<Option<serde_json::Value>, _>("receipt")
                .map(serde_json::from_value)
                .transpose()?,
        },
        admission: serde_json::from_value(serde_json::json!(row.get::<String, _>("admission")))?,
        failure: row
            .get::<Option<String>, _>("failure")
            .map(|v| serde_json::from_value(serde_json::json!(v)))
            .transpose()?,
    })
}
