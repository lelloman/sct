//! Durable metadata-only drafts. Every advance processes a bounded work page.
use crate::{
    auth::{Actor, authorize},
    content_catalog::{edge, lock_content_root, object},
    error::{Error, Result, id, require},
    operations::OperationFence,
    policy::effective,
    roots::{json, lock_root, remember, replay, revision},
    transfer_io::TransferEngine,
    tree_catalog::metadata,
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

pub(crate) async fn check_base(
    db: &mut PgConnection,
    request: &CreateTreeDraftRequest,
) -> Result<EffectivePolicy> {
    let b = &request.base;
    let row = sqlx::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2 AND NOT hidden")
        .bind(b.root_id.as_str())
        .bind(b.directory_id.as_str())
        .fetch_optional(&mut *db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    revision(&row, &b.expected_revision)?;
    require(
        row.get::<String, _>("kind") == "directory"
            && row.get::<String, _>("versioning") == "atomic_directory",
        ErrorCode::InvalidRequest,
    )?;
    require(
        row.get::<Option<String>, _>("current_version_id")
            .as_deref()
            == b.base_version_id.as_ref().map(|v| v.as_str()),
        ErrorCode::RevisionConflict,
    )?;
    let hidden: bool = sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,hidden FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.hidden FROM entries e JOIN a ON e.id=a.parent_id) SELECT bool_or(hidden) FROM a")
        .bind(b.directory_id.as_str()).fetch_one(&mut *db).await?;
    require(!hidden, ErrorCode::NotFound)?;
    let policy = effective(db, b.root_id.as_str(), Some(b.directory_id.as_str())).await?;
    require(
        policy.revision == request.expected_policy_revision,
        ErrorCode::RevisionConflict,
    )?;
    Ok(policy)
}

fn status(row: &PgRow) -> Result<TreeDraftStatus> {
    Ok(TreeDraftStatus {
        id: row.get::<String, _>("id").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        directory_id: row.get::<String, _>("directory_id").parse()?,
        operation_id: row.get::<String, _>("operation_id").parse()?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        phase: serde_json::from_value(json(&row.get::<String, _>("phase"))?)?,
        admission: serde_json::from_value(json(&row.get::<String, _>("admission"))?)?,
        processed_entries: (row.get::<i64, _>("processed") as u64).try_into()?,
        expires_at_ms: (row.get::<i64, _>("expires_ms") as u64).try_into()?,
        result: row
            .get::<Option<serde_json::Value>, _>("receipt")
            .map(serde_json::from_value)
            .transpose()?,
    })
}

impl TransferEngine {
    pub(crate) async fn locked_draft(
        &self,
        db: &mut PgConnection,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<PgRow> {
        authorize(db, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(db, root.as_str()).await?;
        authorize(db, actor, root.as_str(), Capability::Write).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tree_drafts WHERE root_id=$1 AND id=$2 AND principal_id=$3)")
            .bind(root.as_str()).bind(draft.as_str()).bind(&actor.principal).fetch_one(&mut *db).await?;
        require(exists, ErrorCode::NotFound)?;
        sqlx::query("UPDATE transfer_sessions SET state='expired' WHERE id=$1 AND state IN ('open','ready') AND expires_at<=clock_timestamp()")
            .bind(draft.as_str()).execute(&mut *db).await?;
        sqlx::query("UPDATE transfer_sessions SET expires_at=clock_timestamp()+make_interval(secs=>$2) WHERE id=$1 AND state IN ('open','ready') AND expires_at>clock_timestamp()")
            .bind(draft.as_str()).bind(self.expiry_seconds as f64).execute(&mut *db).await?;
        sqlx::query("UPDATE tree_drafts SET state='expired',phase='terminal' WHERE id=$1 AND state IN ('open','ready') AND expires_at<=clock_timestamp() AND NOT EXISTS(SELECT 1 FROM transfer_sessions WHERE id=$1)")
            .bind(draft.as_str()).execute(&mut *db).await?;
        sqlx::query("UPDATE tree_drafts SET expires_at=clock_timestamp()+make_interval(secs=>$2) WHERE id=$1 AND state IN ('open','ready') AND expires_at>clock_timestamp()")
            .bind(draft.as_str()).bind(self.expiry_seconds as f64).execute(&mut *db).await?;
        sqlx::query("UPDATE operations o SET epoch=epoch+1,server_epoch=$2,worker_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=(SELECT operation_id FROM tree_drafts WHERE id=$1) AND state IN ('active','recovering','cleaning_up') AND (server_epoch<>$2 OR worker_expires_at IS NULL OR worker_expires_at<=clock_timestamp())")
            .bind(draft.as_str()).bind(self.writer.epoch).execute(&mut *db).await?;
        Ok(sqlx::query("SELECT d.*,o.epoch,COALESCE(s.admission,'planning') admission,(extract(epoch FROM d.expires_at)*1000)::bigint expires_ms FROM tree_drafts d JOIN operations o ON o.id=d.operation_id LEFT JOIN transfer_sessions s ON s.id=d.id WHERE d.id=$1")
            .bind(draft.as_str()).fetch_one(&mut *db).await?)
    }

    pub async fn create_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &CreateTreeDraftRequest,
    ) -> Result<TreeDraftStatus> {
        require(&request.base.root_id == root, ErrorCode::InvalidRequest)?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        if let Some(r) = sqlx::query(
            "SELECT id,request FROM tree_drafts WHERE root_id=$1 AND principal_id=$2 AND key=$3",
        )
        .bind(root.as_str())
        .bind(&actor.principal)
        .bind(request.idempotency_key.as_str())
        .fetch_optional(&mut *tx)
        .await?
        {
            require(
                r.get::<serde_json::Value, _>("request") == json(request)?,
                ErrorCode::IdempotencyConflict,
            )?;
            let draft = r.get::<String, _>("id").parse()?;
            tx.commit().await?;
            return self.tree_draft_status(actor, root, &draft).await;
        }
        lock_root(&mut tx, root.as_str()).await?;
        let enabled: bool = sqlx::query_scalar("SELECT state='enabled' FROM roots WHERE id=$1")
            .bind(root.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(enabled, ErrorCode::DestinationUnavailable)?;
        check_base(&mut tx, request).await?;
        let draft = id();
        let operation = id();
        sqlx::query("INSERT INTO operations(id,root_id,principal_id,kind,state,phase,server_epoch,worker_expires_at) VALUES($1,$2,$3,'upload','active','draft',$4,clock_timestamp()+interval '30 seconds')")
            .bind(&operation).bind(root.as_str()).bind(&actor.principal).bind(self.writer.epoch).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO tree_drafts(id,root_id,directory_id,operation_id,principal_id,key,request,base_version_id,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,clock_timestamp()+make_interval(secs=>$9))")
            .bind(&draft).bind(root.as_str()).bind(request.base.directory_id.as_str()).bind(&operation).bind(&actor.principal)
            .bind(request.idempotency_key.as_str()).bind(json(request)?).bind(request.base.base_version_id.as_ref().map(|v|v.as_str())).bind(self.expiry_seconds as f64).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO draft_entries(root_id,draft_id,id,name,kind,origin) VALUES($1,$2,$3,'root','directory','base')")
            .bind(root.as_str()).bind(&draft).bind(request.base.directory_id.as_str()).execute(&mut *tx).await?;
        if let Some(base) = &request.base.base_version_id {
            let manifest:String=sqlx::query_scalar("SELECT manifest FROM file_versions WHERE root_id=$1 AND id=$2 AND entry_id=$3 AND kind='directory' AND NOT retired")
                .bind(root.as_str()).bind(base.as_str()).bind(request.base.directory_id.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::RevisionConflict))?;
            sqlx::query("INSERT INTO tree_work(root_id,draft_id,kind,entry_id,manifest) VALUES($1,$2,'directory',$3,$4)")
                .bind(root.as_str()).bind(&draft).bind(request.base.directory_id.as_str()).bind(manifest).execute(&mut *tx).await?;
        } else {
            sqlx::query("UPDATE tree_drafts SET phase='editing' WHERE id=$1")
                .bind(&draft)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.tree_draft_status(actor, root, &draft.parse()?).await
    }

    pub async fn tree_draft_status(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        let result = status(&row)?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn allocate_draft_identities(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
        request: &AllocateDraftIdentitiesRequest,
    ) -> Result<DraftIdentities> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        let scope = format!("tree-identities:{}", draft.as_str());
        if let Some(result) = replay(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            &json(request)?,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(result);
        }
        require(
            row.get::<String, _>("state") == "open" && row.get::<String, _>("phase") == "editing",
            ErrorCode::InvalidRequest,
        )?;
        let mut entry_ids = Vec::new();
        for _ in 0..request.count.get() {
            let identity = id();
            sqlx::query("INSERT INTO draft_identities(root_id,draft_id,id) VALUES($1,$2,$3)")
                .bind(root.as_str())
                .bind(draft.as_str())
                .bind(&identity)
                .execute(&mut *tx)
                .await?;
            entry_ids.push(identity.parse()?);
        }
        let result = DraftIdentities { entry_ids };
        remember(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            json(request)?,
            json(&result)?,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn mutate_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
        request: &DraftMutationPage,
    ) -> Result<TreeDraftStatus> {
        request.validate()?;
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        let scope = format!("tree-mutations:{}", draft.as_str());
        if replay::<bool>(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            &json(request)?,
        )
        .await?
        .is_some()
        {
            tx.commit().await?;
            return self.tree_draft_status(actor, root, draft).await;
        }
        require(
            row.get::<String, _>("state") == "open" && row.get::<String, _>("phase") == "editing",
            ErrorCode::InvalidRequest,
        )?;
        for mutation in &request.mutations {
            match mutation {
                DraftMutation::Remove { entry_id } => {
                    require(
                        entry_id.as_str() != row.get::<String, _>("directory_id"),
                        ErrorCode::InvalidRequest,
                    )?;
                    authorize(&mut tx, actor, root.as_str(), Capability::Delete).await?;
                    let n=sqlx::query("UPDATE draft_entries SET removed=true WHERE root_id=$1 AND draft_id=$2 AND id=$3")
                        .bind(root.as_str()).bind(draft.as_str()).bind(entry_id.as_str()).execute(&mut *tx).await?.rows_affected();
                    require(n == 1, ErrorCode::NotFound)?;
                    sqlx::query("UPDATE tree_drafts SET requires_delete=true WHERE id=$1")
                        .bind(draft.as_str())
                        .execute(&mut *tx)
                        .await?;
                }
                DraftMutation::Upsert {
                    identity,
                    parent_id,
                    name,
                    entry_kind,
                    file,
                } => {
                    let (entry_id, origin) = match identity {
                        DraftEntryIdentity::Existing { entry_id } => (entry_id, "base"),
                        DraftEntryIdentity::New { entry_id } => (entry_id, "new"),
                    };
                    require(
                        entry_id.as_str() != row.get::<String, _>("directory_id"),
                        ErrorCode::InvalidRequest,
                    )?;
                    let prior=sqlx::query("SELECT kind,origin FROM draft_entries WHERE root_id=$1 AND draft_id=$2 AND id=$3")
                        .bind(root.as_str()).bind(draft.as_str()).bind(entry_id.as_str()).fetch_optional(&mut *tx).await?;
                    let kind = if *entry_kind == EntryKind::File {
                        "file"
                    } else {
                        "directory"
                    };
                    if let Some(prior) = prior {
                        require(
                            prior.get::<String, _>("origin") == origin
                                && prior.get::<String, _>("kind") == kind,
                            ErrorCode::InvalidRequest,
                        )?;
                    } else {
                        require(origin == "new", ErrorCode::InvalidRequest)?;
                        let allocated:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM draft_identities WHERE root_id=$1 AND draft_id=$2 AND id=$3)")
                            .bind(root.as_str()).bind(draft.as_str()).bind(entry_id.as_str()).fetch_one(&mut *tx).await?;
                        require(allocated, ErrorCode::InvalidRequest)?;
                    }
                    let manifest = if let Some(file) = file {
                        Some(store_file(&mut tx, root.as_str(), file).await?)
                    } else {
                        None
                    };
                    sqlx::query("INSERT INTO draft_entries(root_id,draft_id,id,parent_id,name,kind,manifest,origin) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(root_id,draft_id,id) DO UPDATE SET parent_id=$4,name=$5,manifest=$7,removed=false,built=false")
                        .bind(root.as_str()).bind(draft.as_str()).bind(entry_id.as_str()).bind(parent_id.as_str()).bind(name.as_str()).bind(kind).bind(manifest).bind(origin).execute(&mut *tx).await?;
                }
            }
        }
        remember(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            json(request)?,
            json(&true)?,
        )
        .await?;
        tx.commit().await?;
        self.tree_draft_status(actor, root, draft).await
    }

    pub async fn tree_draft_page(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
        manifest: &ManifestId,
        bytes: &[u8],
    ) -> Result<()> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        require(
            row.get::<String, _>("state") == "open" && row.get::<String, _>("phase") == "editing",
            ErrorCode::InvalidRequest,
        )?;
        let fence = OperationFence {
            id: row.get("operation_id"),
            root: root.as_str().into(),
            epoch: row.get("epoch"),
        };
        tx.commit().await?;
        self.writer
            .store_manifest_page(&fence, manifest, bytes)
            .await
    }

    pub async fn freeze_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        require(
            row.get::<String, _>("state") == "open",
            ErrorCode::InvalidRequest,
        )?;
        if row.get::<String, _>("phase") == "editing" {
            let request: CreateTreeDraftRequest = serde_json::from_value(row.get("request"))?;
            check_base(&mut tx, &request).await?;
            let removed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM draft_entries WHERE root_id=$1 AND draft_id=$2 AND origin='base' AND removed)")
                .bind(root.as_str()).bind(draft.as_str()).fetch_one(&mut *tx).await?;
            if removed || row.get::<bool, _>("requires_delete") {
                authorize(&mut tx, actor, root.as_str(), Capability::Delete).await?;
            }
            sqlx::query("UPDATE tree_drafts SET phase='building',requires_delete=requires_delete OR $2 WHERE id=$1")
                .bind(draft.as_str()).bind(removed).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO tree_work(root_id,draft_id,kind,position) VALUES($1,$2,'validate',$3)",
            )
            .bind(root.as_str())
            .bind(draft.as_str())
            .bind(serde_json::json!({"after":""}))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.advance_tree_draft(actor, root, draft).await
    }

    pub async fn advance_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        require(
            row.get::<String, _>("state") == "open",
            ErrorCode::InvalidRequest,
        )?;
        let work = sqlx::query(
            "SELECT * FROM tree_work WHERE root_id=$1 AND draft_id=$2 ORDER BY sequence LIMIT 1",
        )
        .bind(root.as_str())
        .bind(draft.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(work) = work {
            let sequence: i64 = work.get("sequence");
            match work.get::<String, _>("kind").as_str() {
                "directory" => {
                    let manifest: DirectoryManifest = metadata(
                        &mut tx,
                        root.as_str(),
                        &work.get::<String, _>("manifest"),
                        "directory",
                    )
                    .await?;
                    manifest.validate()?;
                    if let Some(tree) = manifest.tree {
                        sqlx::query("INSERT INTO tree_work(root_id,draft_id,kind,entry_id,manifest) VALUES($1,$2,'page',$3,$4)")
                            .bind(root.as_str()).bind(draft.as_str()).bind(work.get::<String,_>("entry_id")).bind(tree.as_str()).execute(&mut *tx).await?;
                    }
                }
                "page" => {
                    let page: DirectoryPage = metadata(
                        &mut tx,
                        root.as_str(),
                        &work.get::<String, _>("manifest"),
                        "directory_page",
                    )
                    .await?;
                    page.validate()?;
                    let parent: String = work.get("entry_id");
                    match page {
                        DirectoryPage::DirectoryBranch { pages, .. } => {
                            for page in pages {
                                sqlx::query("INSERT INTO tree_work(root_id,draft_id,kind,entry_id,manifest) VALUES($1,$2,'page',$3,$4)")
                                .bind(root.as_str()).bind(draft.as_str()).bind(&parent).bind(page.digest.as_str()).execute(&mut *tx).await?;
                            }
                        }
                        DirectoryPage::DirectoryEntries { entries, .. } => {
                            let request: CreateTreeDraftRequest =
                                serde_json::from_value(row.get("request"))?;
                            for entry in &entries {
                                let directory = entry.kind == EntryKind::Directory;
                                sqlx::query("INSERT INTO draft_entries(root_id,draft_id,id,parent_id,name,kind,manifest,origin,removed) VALUES($1,$2,$3,$4,$5,$6,$7,'base',$8)")
                                    .bind(root.as_str()).bind(draft.as_str()).bind(entry.id.as_str()).bind(&parent).bind(entry.name.as_str()).bind(if directory {"directory"}else{"file"}).bind(entry.manifest.as_str())
                                    .bind(request.base.mode == DraftMode::CompleteReplacement).execute(&mut *tx).await?;
                                if directory {
                                    sqlx::query("INSERT INTO tree_work(root_id,draft_id,kind,entry_id,manifest) VALUES($1,$2,'directory',$3,$4)")
                                    .bind(root.as_str()).bind(draft.as_str()).bind(entry.id.as_str()).bind(entry.manifest.as_str()).execute(&mut *tx).await?;
                                }
                            }
                            sqlx::query(
                                "UPDATE tree_drafts SET processed=processed+$2 WHERE id=$1",
                            )
                            .bind(draft.as_str())
                            .bind(entries.len() as i64)
                            .execute(&mut *tx)
                            .await?;
                        }
                    }
                }
                "validate" => {
                    let position: serde_json::Value = work.get("position");
                    let after = position["after"]
                        .as_str()
                        .ok_or(Error(ErrorCode::InternalError))?;
                    let entries:Vec<String>=sqlx::query_scalar("SELECT id FROM draft_entries WHERE root_id=$1 AND draft_id=$2 AND id>$3 AND NOT removed ORDER BY id LIMIT 64")
                        .bind(root.as_str()).bind(draft.as_str()).bind(after).fetch_all(&mut *tx).await?;
                    let boundary_length:i64=sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,name FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.name FROM entries e JOIN a ON e.id=a.parent_id) SELECT COALESCE(sum(char_length(name)+1) FILTER(WHERE parent_id IS NOT NULL),0)::bigint FROM a")
                        .bind(row.get::<String,_>("directory_id")).fetch_one(&mut *tx).await?;
                    for entry in &entries {
                        validate_entry(
                            &mut tx,
                            root.as_str(),
                            draft.as_str(),
                            entry,
                            &row.get::<String, _>("directory_id"),
                            boundary_length as usize,
                        )
                        .await?;
                    }
                    if let Some(last) = entries.last() {
                        sqlx::query("UPDATE tree_work SET position=$2 WHERE sequence=$1")
                            .bind(sequence)
                            .bind(serde_json::json!({"after":last}))
                            .execute(&mut *tx)
                            .await?;
                        tx.commit().await?;
                        return self.tree_draft_status(actor, root, draft).await;
                    }
                }
                _ => return Err(Error(ErrorCode::InvalidRequest)),
            }
            sqlx::query("DELETE FROM tree_work WHERE root_id=$1 AND draft_id=$2 AND sequence=$3")
                .bind(root.as_str())
                .bind(draft.as_str())
                .bind(sequence)
                .execute(&mut *tx)
                .await?;
        } else if row.get::<String, _>("phase") == "building" {
            crate::tree_build::build_step(&mut tx, actor, &row).await?;
        }
        sqlx::query("UPDATE tree_drafts SET phase='editing' WHERE id=$1 AND phase='initializing' AND NOT EXISTS(SELECT 1 FROM tree_work WHERE draft_id=$1)")
            .bind(draft.as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        self.tree_draft_status(actor, root, draft).await
    }

    pub async fn abort_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus> {
        let mut tx = self.writer.transaction().await?;
        let row = self.locked_draft(&mut tx, actor, root, draft).await?;
        require(
            row.get::<String, _>("state") != "committed",
            ErrorCode::InvalidRequest,
        )?;
        let transfer: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM transfer_sessions WHERE id=$1)")
                .bind(draft.as_str())
                .fetch_one(&mut *tx)
                .await?;
        if transfer {
            tx.commit().await?;
            self.abort_upload(actor, root, draft).await?;
            tx = self.writer.transaction().await?;
            self.locked_draft(&mut tx, actor, root, draft).await?;
        } else {
            sqlx::query("UPDATE operations SET state='aborted',worker_expires_at=NULL WHERE id=$1")
                .bind(row.get::<String, _>("operation_id"))
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE tree_drafts SET state='aborted',phase='terminal' WHERE id=$1")
            .bind(draft.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.tree_draft_status(actor, root, draft).await
    }
}

async fn validate_entry(
    db: &mut PgConnection,
    root: &str,
    draft: &str,
    entry: &str,
    boundary: &str,
    boundary_length: usize,
) -> Result<()> {
    let mut current = entry.to_owned();
    let mut seen = std::collections::BTreeSet::new();
    let mut length = boundary_length;
    loop {
        require(seen.insert(current.clone()), ErrorCode::InvalidManifest)?;
        let row=sqlx::query("SELECT parent_id,name,kind,removed FROM draft_entries WHERE root_id=$1 AND draft_id=$2 AND id=$3")
            .bind(root).bind(draft).bind(&current).fetch_optional(&mut *db).await?.ok_or(Error(ErrorCode::InvalidManifest))?;
        if current != entry {
            require(
                row.get::<String, _>("kind") == "directory",
                ErrorCode::InvalidManifest,
            )?;
        }
        if row.get::<bool, _>("removed") {
            sqlx::query(
                "UPDATE draft_entries SET removed=true WHERE root_id=$1 AND draft_id=$2 AND id=$3",
            )
            .bind(root)
            .bind(draft)
            .bind(entry)
            .execute(&mut *db)
            .await?;
            return Ok(());
        }
        let parent: Option<String> = row.get("parent_id");
        if let Some(parent) = parent {
            length += row.get::<String, _>("name").chars().count() + 1;
            require(length <= 4096, ErrorCode::InvalidPath)?;
            current = parent;
        } else {
            return require(current == boundary, ErrorCode::InvalidManifest);
        }
    }
}

pub(crate) async fn store_file(
    db: &mut PgConnection,
    root: &str,
    file: &FileManifest,
) -> Result<String> {
    file.validate()?;
    let body = canonical(file)?;
    let manifest = digest(&body);
    object(
        db,
        root,
        &manifest,
        "file",
        file.length.get(),
        Some(file.height),
        Some(&body),
    )
    .await?;
    if let Some(tree) = &file.tree {
        object(
            db,
            root,
            tree.as_str(),
            "page",
            file.length.get(),
            Some(file.height),
            None,
        )
        .await?;
        edge(
            db,
            root,
            &manifest,
            0,
            tree.as_str(),
            file.length.get(),
            file.chunks.get(),
        )
        .await?;
    }
    Ok(manifest)
}
