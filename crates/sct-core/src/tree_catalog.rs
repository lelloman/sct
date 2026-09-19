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
            "SELECT manifest FROM file_versions WHERE root_id=$1 AND id=$2 AND kind='directory' AND NOT retired",
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
