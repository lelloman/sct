//! Incremental canonical tree materialization and metadata-first admission.
use crate::{
    auth::{Actor, authorize},
    content_catalog::{edge, object},
    error::{Error, Result, require},
    roots::json,
    transfer_io::TransferEngine,
    tree_catalog::metadata,
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

impl TransferEngine {
    pub(crate) async fn cleanup_terminal_tree_work(&self) -> Result<()> {
        let drafts = {
            let mut tx = self.writer.transaction().await?;
            sqlx::query("SELECT d.id,d.root_id FROM tree_drafts d WHERE d.state IN ('committed','aborted','expired') AND (EXISTS(SELECT 1 FROM draft_entries e WHERE e.root_id=d.root_id AND e.draft_id=d.id) OR EXISTS(SELECT 1 FROM draft_identities i WHERE i.root_id=d.root_id AND i.draft_id=d.id) OR EXISTS(SELECT 1 FROM tree_work w WHERE w.root_id=d.root_id AND w.draft_id=d.id)) ORDER BY d.root_id,d.id LIMIT 8").fetch_all(&mut *tx).await?
        };
        for row in drafts {
            let root: String = row.get("root_id");
            let draft: String = row.get("id");
            let mut tx = self.writer.transaction().await?;
            crate::content_catalog::lock_content_root(&mut tx, &root).await?;
            // Terminal state is irreversible. These are temporary metadata rows;
            // payload deletion still requires its operation and generation fence.
            for table in [
                "tree_work",
                "draft_build_state",
                "draft_build_pages",
                "draft_identities",
                "draft_entries",
            ] {
                let n=sqlx::query(&format!("DELETE FROM {table} WHERE ctid IN (SELECT ctid FROM {table} WHERE root_id=$1 AND draft_id=$2 LIMIT 512)"))
                    .bind(&root).bind(&draft).execute(&mut *tx).await?.rows_affected();
                if n == 512 {
                    break;
                }
            }
            tx.commit().await?;
        }
        Ok(())
    }
    pub async fn admit_tree_draft(
        &self,
        actor: &Actor,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus> {
        let state = self.tree_draft_status(actor, root, draft).await?;
        if matches!(
            state.phase,
            TreeDraftPhase::Initializing | TreeDraftPhase::Editing
        ) {
            return Err(Error(ErrorCode::InvalidRequest));
        }
        if state.phase == TreeDraftPhase::Building {
            let state = self.advance_tree_draft(actor, root, draft).await?;
            if state.phase == TreeDraftPhase::Building {
                return Ok(state);
            }
        }
        self.admit_upload(actor, root, draft).await?;
        self.tree_draft_status(actor, root, draft).await
    }
}

pub(crate) async fn build_step(db: &mut PgConnection, actor: &Actor, row: &PgRow) -> Result<()> {
    let root: String = row.get("root_id");
    let draft: String = row.get("id");
    if row.get::<bool, _>("requires_delete") {
        authorize(db, actor, &root, Capability::Delete).await?;
    }
    let request: CreateTreeDraftRequest = serde_json::from_value(row.get("request"))?;
    crate::tree_draft::check_base(db, &request).await?;
    let state = sqlx::query("SELECT * FROM draft_build_state WHERE root_id=$1 AND draft_id=$2")
        .bind(&root)
        .bind(&draft)
        .fetch_optional(&mut *db)
        .await?;
    let (directory, height, after) = if let Some(state) = state {
        (
            state.get::<String, _>("directory_id"),
            state.get::<i32, _>("height") as u32,
            state.get::<String, _>("after_name"),
        )
    } else {
        let directory:String=sqlx::query_scalar("SELECT e.id FROM draft_entries e WHERE e.root_id=$1 AND e.draft_id=$2 AND e.kind='directory' AND NOT e.built AND NOT e.removed AND NOT EXISTS(SELECT 1 FROM draft_entries c WHERE c.root_id=e.root_id AND c.draft_id=e.draft_id AND c.parent_id=e.id AND c.kind='directory' AND NOT c.removed AND NOT c.built) ORDER BY e.id LIMIT 1")
            .bind(&root).bind(&draft).fetch_optional(&mut *db).await?.ok_or(Error(ErrorCode::InvalidManifest))?;
        sqlx::query(
            "INSERT INTO draft_build_state(root_id,draft_id,directory_id) VALUES($1,$2,$3)",
        )
        .bind(&root)
        .bind(&draft)
        .bind(&directory)
        .execute(&mut *db)
        .await?;
        (directory, 0, String::new())
    };
    if height == 0 {
        let rows=sqlx::query("SELECT id,name,kind,manifest FROM draft_entries WHERE root_id=$1 AND draft_id=$2 AND parent_id=$3 AND NOT removed AND name>$4 ORDER BY name LIMIT 128")
            .bind(&root).bind(&draft).bind(&directory).bind(&after).fetch_all(&mut *db).await?;
        if rows.is_empty() {
            return next_level(db, row, &directory, height).await;
        }
        let mut entries = Vec::new();
        for r in rows {
            let manifest: String = r
                .get::<Option<String>, _>("manifest")
                .ok_or(Error(ErrorCode::InvalidManifest))?;
            let (kind, length, chunks) = if r.get::<String, _>("kind") == "file" {
                let file: FileManifest = metadata(db, &root, &manifest, "file").await?;
                file.validate()?;
                (EntryKind::File, file.length, file.chunks)
            } else {
                let directory: DirectoryManifest =
                    metadata(db, &root, &manifest, "directory").await?;
                directory.validate()?;
                (EntryKind::Directory, directory.length, directory.chunks)
            };
            entries.push(DirectoryEntry {
                id: r.get::<String, _>("id").parse()?,
                name: r.get::<String, _>("name").parse()?,
                kind,
                manifest: manifest.parse()?,
                length,
                chunks,
            });
        }
        let page = DirectoryPage::DirectoryEntries { format: 1, entries };
        save_page(db, &root, &draft, &directory, &page).await?;
    } else {
        let pages:Vec<serde_json::Value>=sqlx::query_scalar("SELECT reference FROM draft_build_pages WHERE root_id=$1 AND draft_id=$2 AND directory_id=$3 AND height=$4 AND first>$5 ORDER BY first LIMIT 64")
            .bind(&root).bind(&draft).bind(&directory).bind((height-1) as i32).bind(&after).fetch_all(&mut *db).await?;
        if pages.is_empty() {
            return next_level(db, row, &directory, height).await;
        }
        let pages: Vec<DirectoryPageReference> = pages
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        save_page(
            db,
            &root,
            &draft,
            &directory,
            &DirectoryPage::DirectoryBranch {
                format: 1,
                height,
                pages,
            },
        )
        .await?;
    }
    Ok(())
}

async fn save_page(
    db: &mut PgConnection,
    root: &str,
    draft: &str,
    directory: &str,
    page: &DirectoryPage,
) -> Result<()> {
    page.validate()?;
    let body = canonical(page)?;
    let digest = digest(&body);
    let (height, first, last, entries, length, chunks, children) = match page {
        DirectoryPage::DirectoryEntries { entries, .. } => (
            0,
            entries.first().unwrap().name.clone(),
            entries.last().unwrap().name.clone(),
            entries.len() as u64,
            entries.iter().map(|e| e.length.get()).sum::<u64>(),
            entries.iter().map(|e| e.chunks.get()).sum::<u64>(),
            entries
                .iter()
                .map(|e| (e.manifest.as_str(), e.length.get(), e.chunks.get()))
                .collect::<Vec<_>>(),
        ),
        DirectoryPage::DirectoryBranch { height, pages, .. } => (
            *height,
            pages.first().unwrap().first.clone(),
            pages.last().unwrap().last.clone(),
            pages.iter().map(|p| p.entries.get()).sum::<u64>(),
            pages.iter().map(|p| p.length.get()).sum::<u64>(),
            pages.iter().map(|p| p.chunks.get()).sum::<u64>(),
            pages
                .iter()
                .map(|p| (p.digest.as_str(), p.length.get(), p.chunks.get()))
                .collect::<Vec<_>>(),
        ),
    };
    object(
        db,
        root,
        &digest,
        "directory_page",
        length,
        Some(height),
        Some(&body),
    )
    .await?;
    for (i, (child, length, chunks)) in children.into_iter().enumerate() {
        edge(db, root, &digest, i, child, length, chunks).await?;
    }
    let reference = DirectoryPageReference {
        digest: digest.parse()?,
        first: first.clone(),
        last: last.clone(),
        entries: entries.try_into()?,
        length: length.try_into()?,
        chunks: chunks.try_into()?,
    };
    sqlx::query("INSERT INTO draft_build_pages(root_id,draft_id,directory_id,height,first,reference) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(root).bind(draft).bind(directory).bind(height as i32).bind(first.as_str()).bind(json(&reference)?).execute(&mut *db).await?;
    sqlx::query("UPDATE draft_build_state SET after_name=$3 WHERE root_id=$1 AND draft_id=$2")
        .bind(root)
        .bind(draft)
        .bind(last.as_str())
        .execute(&mut *db)
        .await?;
    Ok(())
}

async fn next_level(
    db: &mut PgConnection,
    row: &PgRow,
    directory: &str,
    height: u32,
) -> Result<()> {
    let root: String = row.get("root_id");
    let draft: String = row.get("id");
    let pages:Vec<serde_json::Value>=sqlx::query_scalar("SELECT reference FROM draft_build_pages WHERE root_id=$1 AND draft_id=$2 AND directory_id=$3 AND height=$4 ORDER BY first LIMIT 2")
        .bind(&root).bind(&draft).bind(directory).bind(height as i32).fetch_all(&mut *db).await?;
    if pages.len() > 1 {
        require(height < MANIFEST_HEIGHT, ErrorCode::InvalidManifest)?;
        sqlx::query("UPDATE draft_build_state SET height=height+1,after_name='' WHERE root_id=$1 AND draft_id=$2").bind(&root).bind(&draft).execute(db).await?;
        return Ok(());
    }
    let reference: Option<DirectoryPageReference> = pages
        .into_iter()
        .next()
        .map(serde_json::from_value)
        .transpose()?;
    let manifest = DirectoryManifest {
        format: 1,
        kind: DirectoryObjectKind::Directory,
        entries: reference
            .as_ref()
            .map(|p| p.entries)
            .unwrap_or(0.try_into()?),
        length: reference
            .as_ref()
            .map(|p| p.length)
            .unwrap_or(0.try_into()?),
        chunks: reference
            .as_ref()
            .map(|p| p.chunks)
            .unwrap_or(0.try_into()?),
        height,
        tree: reference.as_ref().map(|p| p.digest.clone()),
    };
    manifest.validate()?;
    let body = canonical(&manifest)?;
    let digest = digest(&body);
    object(
        db,
        &root,
        &digest,
        "directory",
        manifest.length.get(),
        Some(height),
        Some(&body),
    )
    .await?;
    if let Some(reference) = reference {
        edge(
            db,
            &root,
            &digest,
            0,
            reference.digest.as_str(),
            reference.length.get(),
            reference.chunks.get(),
        )
        .await?;
    }
    sqlx::query("UPDATE draft_entries SET manifest=$4,built=true WHERE root_id=$1 AND draft_id=$2 AND id=$3")
        .bind(&root).bind(&draft).bind(directory).bind(&digest).execute(&mut *db).await?;
    sqlx::query("DELETE FROM draft_build_state WHERE root_id=$1 AND draft_id=$2")
        .bind(&root)
        .bind(&draft)
        .execute(&mut *db)
        .await?;
    if directory == row.get::<String, _>("directory_id") {
        sqlx::query("UPDATE tree_drafts SET manifest=$2,phase='planning' WHERE id=$1")
            .bind(&draft)
            .bind(&digest)
            .execute(&mut *db)
            .await?;
        sqlx::query("INSERT INTO transfer_sessions(id,root_id,operation_id,principal_id,key,request,manifest,expires_at) SELECT id,root_id,operation_id,principal_id,'tree-'||id,request,$2,expires_at FROM tree_drafts WHERE id=$1")
            .bind(&draft).bind(&digest).execute(&mut *db).await?;
        sqlx::query("INSERT INTO session_objects(root_id,session_id,digest) VALUES($1,$2,$3)")
            .bind(&root)
            .bind(&draft)
            .bind(digest)
            .execute(&mut *db)
            .await?;
        sqlx::query("UPDATE operations SET phase='planning' WHERE id=$1")
            .bind(row.get::<String, _>("operation_id"))
            .execute(&mut *db)
            .await?;
    }
    Ok(())
}
