//! Snapshot namespace lookup follows immutable pages, never mutable descendants.
use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, require},
    lease::Writer,
};
use sct_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::PgConnection;

pub(crate) fn path_parts(path: &RootPath) -> Result<Vec<EntryName>> {
    if path.as_str() == "/" {
        return Ok(vec![]);
    }
    path.as_str()[1..]
        .split('/')
        .map(|s| s.parse().map_err(Into::into))
        .collect()
}
pub(crate) async fn resolve(
    db: &mut PgConnection,
    root: &str,
    manifest: &str,
    path: &[EntryName],
) -> Result<DirectoryEntry> {
    require(!path.is_empty(), ErrorCode::InvalidRequest)?;
    let mut manifest = manifest.to_owned();
    for (index, name) in path.iter().enumerate() {
        let entry = child(db, root, &manifest, name).await?;
        if index + 1 == path.len() {
            return Ok(entry);
        }
        require(entry.kind == EntryKind::Directory, ErrorCode::NotFound)?;
        manifest = entry.manifest.as_str().to_owned();
    }
    unreachable!()
}

pub(crate) async fn version_manifest(
    db: &mut PgConnection,
    root: &str,
    version: &str,
) -> Result<String> {
    sqlx::query_scalar("SELECT manifest FROM file_versions WHERE root_id=$1 AND id=$2 AND kind='directory' AND NOT retired AND NOT entry_purged(entry_id)")
        .bind(root).bind(version).fetch_optional(db).await?.ok_or(Error(ErrorCode::NotFound))
}

pub(crate) async fn metadata<T: Serialize + DeserializeOwned>(
    db: &mut PgConnection,
    root: &str,
    manifest: &str,
    kind: &str,
) -> Result<T> {
    let body: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT body FROM content_objects WHERE root_id=$1 AND digest=$2 AND kind=$3",
    )
    .bind(root)
    .bind(manifest)
    .bind(kind)
    .fetch_optional(db)
    .await?
    .flatten();
    let body = body.ok_or(Error(ErrorCode::InvalidManifest))?;
    require(digest(&body) == manifest, ErrorCode::IntegrityFailure)?;
    parse_canonical(&body).map_err(|_| Error(ErrorCode::InvalidManifest))
}

pub(crate) async fn child(
    db: &mut PgConnection,
    root: &str,
    directory: &str,
    name: &EntryName,
) -> Result<DirectoryEntry> {
    let directory: DirectoryManifest = metadata(db, root, directory, "directory").await?;
    directory.validate()?;
    let mut next = directory.tree.ok_or(Error(ErrorCode::NotFound))?;
    let mut height = directory.height;
    loop {
        let page: DirectoryPage = metadata(db, root, next.as_str(), "directory_page").await?;
        page.validate()?;
        match page {
            DirectoryPage::DirectoryEntries { entries, .. } => {
                require(height == 0, ErrorCode::InvalidManifest)?;
                return entries
                    .into_iter()
                    .find(|e| &e.name == name)
                    .ok_or(Error(ErrorCode::NotFound));
            }
            DirectoryPage::DirectoryBranch {
                height: actual,
                pages,
                ..
            } => {
                require(height > 0 && actual == height, ErrorCode::InvalidManifest)?;
                next = pages
                    .into_iter()
                    .find(|p| p.first.as_str() <= name.as_str() && p.last.as_str() >= name.as_str())
                    .ok_or(Error(ErrorCode::NotFound))?
                    .digest;
                height -= 1;
            }
        }
    }
}

impl Writer {
    pub async fn current_snapshot(
        &self,
        actor: &Actor,
        root: &RootId,
        directory: &EntryId,
    ) -> Result<VersionId> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let version:Option<String>=sqlx::query_scalar("SELECT current_version_id FROM entries WHERE root_id=$1 AND id=$2 AND kind='directory' AND versioning='atomic_directory' AND NOT hidden")
            .bind(root.as_str()).bind(directory.as_str()).fetch_optional(&mut *tx).await?.flatten();
        let version = version.ok_or(Error(ErrorCode::NotFound))?;
        let hidden:bool=sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,hidden FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.hidden FROM entries e JOIN a ON e.id=a.parent_id) SELECT bool_or(hidden) FROM a")
            .bind(directory.as_str()).fetch_one(&mut *tx).await?;
        require(!hidden, ErrorCode::NotFound)?;
        Ok(version.parse()?)
    }
    pub async fn snapshot_entries(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        query: &SnapshotListQuery,
    ) -> Result<SnapshotEntryPage> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let manifest = version_manifest(&mut tx, root.as_str(), version.as_str()).await?;
        let binding = serde_json::json!({"root":root,"version":version,"path":query.path,"limit":query.limit});
        let after: String = if let Some(cursor) = &query.cursor {
            crate::paging::consume(&mut tx, actor, "snapshot_entries", cursor, &binding).await?
        } else {
            String::new()
        };
        let path = path_parts(&query.path)?;
        let manifest = if path.is_empty() {
            manifest
        } else {
            let entry = resolve(&mut tx, root.as_str(), &manifest, &path).await?;
            require(entry.kind == EntryKind::Directory, ErrorCode::NotFound)?;
            entry.manifest.as_str().to_owned()
        };
        let directory: DirectoryManifest =
            metadata(&mut tx, root.as_str(), &manifest, "directory").await?;
        directory.validate()?;
        let mut stack = directory
            .tree
            .map(|id| vec![(id, directory.height)])
            .unwrap_or_default();
        let mut entries = Vec::new();
        let limit = query.limit.get() as usize;
        while let Some((manifest, height)) = stack.pop() {
            let page: DirectoryPage =
                metadata(&mut tx, root.as_str(), manifest.as_str(), "directory_page").await?;
            page.validate()?;
            match page {
                DirectoryPage::DirectoryEntries {
                    entries: children, ..
                } => {
                    require(height == 0, ErrorCode::InvalidManifest)?;
                    for entry in children {
                        if entry.name.as_str() > after.as_str() {
                            entries.push(entry);
                            if entries.len() > limit {
                                break;
                            }
                        }
                    }
                }
                DirectoryPage::DirectoryBranch {
                    height: actual,
                    pages,
                    ..
                } => {
                    require(height > 0 && actual == height, ErrorCode::InvalidManifest)?;
                    for page in pages.into_iter().rev() {
                        if page.last.as_str() > after.as_str() {
                            stack.push((page.digest, height - 1));
                        }
                    }
                }
            }
            if entries.len() > limit {
                break;
            }
        }
        let next_cursor = if entries.len() > limit {
            entries.truncate(limit);
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    "snapshot_entries",
                    binding,
                    serde_json::json!(entries.last().unwrap().name.as_str()),
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(SnapshotEntryPage {
            version_id: version.clone(),
            entries,
            next_cursor,
        })
    }
    /// Resolve a nonempty relative path within a retained directory snapshot.
    /// At most nine bounded metadata pages are read for each path component.
    pub async fn snapshot_entry(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        path: &[EntryName],
    ) -> Result<DirectoryEntry> {
        require(
            !path.is_empty()
                && path
                    .iter()
                    .map(|p| p.as_str().chars().count() + 1)
                    .sum::<usize>()
                    <= 4096,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let mut manifest: String = sqlx::query_scalar(
            "SELECT manifest FROM file_versions WHERE root_id=$1 AND id=$2 AND kind='directory' AND NOT retired AND NOT entry_purged(entry_id)",
        ).bind(root.as_str()).bind(version.as_str()).fetch_optional(&mut *tx).await?
            .ok_or(Error(ErrorCode::NotFound))?;
        for (index, name) in path.iter().enumerate() {
            let entry = child(&mut tx, root.as_str(), &manifest, name).await?;
            if index + 1 == path.len() {
                tx.commit().await?;
                return Ok(entry);
            }
            require(entry.kind == EntryKind::Directory, ErrorCode::NotFound)?;
            manifest = entry.manifest.as_str().to_owned();
        }
        unreachable!("nonempty path validated")
    }
}
