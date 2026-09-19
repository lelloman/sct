//! Tree plans and explicit lifecycle operations, independent of server internals.
use crate::{Client, Error, UploadJob};
use reqwest::Method;
use sct_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

impl Client {
    pub async fn create_tree_draft(
        &self,
        root: &RootId,
        request: &CreateTreeDraftRequest,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/roots/{}/tree-drafts", root.as_str()),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn advance_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/advance",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn freeze_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/freeze",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn abort_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/abort",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn admit_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/admit",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn commit_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
    ) -> Result<CommitReceipt, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/commit",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn allocate_tree_identities(
        &self,
        root: &RootId,
        draft: &UploadId,
        request: &AllocateDraftIdentitiesRequest,
    ) -> Result<DraftIdentities, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/identities",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn mutate_tree_draft(
        &self,
        root: &RootId,
        draft: &UploadId,
        page: &DraftMutationPage,
    ) -> Result<TreeDraftStatus, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/mutations",
                root.as_str(),
                draft.as_str()
            ),
            vec![],
            Some(serde_json::to_value(page)?),
        )
        .await
    }
    pub async fn hide_entry(
        &self,
        root: &RootId,
        entry: &EntryId,
        request: &HideEntryRequest,
    ) -> Result<EntryInspection, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/entries/{}/hide",
                root.as_str(),
                entry.as_str()
            ),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn current_snapshot(
        &self,
        root: &RootId,
        entry: &EntryId,
    ) -> Result<VersionId, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/entries/{}/snapshot",
                root.as_str(),
                entry.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn snapshot_entries(
        &self,
        root: &RootId,
        version: &VersionId,
        query: &SnapshotListQuery,
    ) -> Result<SnapshotEntryPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/snapshots/{}/entries",
                root.as_str(),
                version.as_str()
            ),
            snapshot_query(query),
            None,
        )
        .await
    }
    pub async fn snapshot_entry(
        &self,
        root: &RootId,
        version: &VersionId,
        path: &RootPath,
    ) -> Result<DirectoryEntry, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/snapshots/{}/entry",
                root.as_str(),
                version.as_str()
            ),
            vec![("path", path.as_str().into())],
            None,
        )
        .await
    }
    pub async fn snapshot_manifest(
        &self,
        root: &RootId,
        version: &VersionId,
        path: &RootPath,
    ) -> Result<FileManifest, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/snapshots/{}/manifest",
                root.as_str(),
                version.as_str()
            ),
            vec![("path", path.as_str().into())],
            None,
        )
        .await
    }
    pub async fn preview_change(
        &self,
        root: &RootId,
        request: &PreviewChangeRequest,
    ) -> Result<ChangePreview, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/roots/{}/policy-previews", root.as_str()),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn inspect_preview(
        &self,
        root: &RootId,
        preview: &PreviewId,
    ) -> Result<ChangePreview, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/policy-previews/{}",
                root.as_str(),
                preview.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn advance_preview(
        &self,
        root: &RootId,
        preview: &PreviewId,
    ) -> Result<ChangePreview, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/policy-previews/{}/advance",
                root.as_str(),
                preview.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn preview_versions(
        &self,
        root: &RootId,
        preview: &PreviewId,
        query: &ListRequest,
    ) -> Result<PreviewVersionPage, Error> {
        self.request(
            Method::GET,
            &format!(
                "/api/v1/roots/{}/policy-previews/{}/versions",
                root.as_str(),
                preview.as_str()
            ),
            list_query(query),
            None,
        )
        .await
    }
    pub async fn apply_change(
        &self,
        root: &RootId,
        request: &ApplyChangeRequest,
    ) -> Result<LifecycleJob, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/roots/{}/policy-jobs", root.as_str()),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn purge(
        &self,
        root: &RootId,
        request: &PurgeRequest,
    ) -> Result<LifecycleJob, Error> {
        self.request(
            Method::POST,
            &format!("/api/v1/roots/{}/purge-jobs", root.as_str()),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn lifecycle_job(
        &self,
        root: &RootId,
        job: &OperationId,
    ) -> Result<LifecycleJob, Error> {
        self.request(
            Method::GET,
            &format!("/api/v1/roots/{}/jobs/{}", root.as_str(), job.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn advance_lifecycle_job(
        &self,
        root: &RootId,
        job: &OperationId,
    ) -> Result<LifecycleJob, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/jobs/{}/advance",
                root.as_str(),
                job.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn resume_lifecycle_job(
        &self,
        root: &RootId,
        job: &OperationId,
    ) -> Result<LifecycleJob, Error> {
        self.request(
            Method::POST,
            &format!(
                "/api/v1/roots/{}/jobs/{}/resume",
                root.as_str(),
                job.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    pub async fn tree_draft_page(
        &self,
        root: &RootId,
        draft: &UploadId,
        page: &ManifestPage,
    ) -> Result<(), Error> {
        page.validate().map_err(invalid)?;
        let bytes = canonical(page).map_err(invalid)?;
        self.put_bytes(
            &format!(
                "/api/v1/roots/{}/tree-drafts/{}/pages/{}",
                root.as_str(),
                draft.as_str(),
                digest(&bytes)
            ),
            bytes,
        )
        .await
    }
}
fn list_query(q: &ListRequest) -> Vec<(&'static str, String)> {
    let mut result = vec![("limit", q.limit.get().to_string())];
    if let Some(c) = &q.cursor {
        result.push(("cursor", c.as_str().into()))
    }
    result
}
fn snapshot_query(q: &SnapshotListQuery) -> Vec<(&'static str, String)> {
    let mut result = list_query(&ListRequest {
        limit: q.limit,
        cursor: q.cursor.clone(),
    });
    result.push(("path", q.path.as_str().into()));
    result
}
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Local(e.to_string())
}

/// An explicit tree edit. Sources must be stable, caller-captured files.
/// New identities come from allocate_tree_identities on the same draft.
pub enum TreeMutation {
    File {
        identity: DraftEntryIdentity,
        parent_id: EntryId,
        name: EntryName,
        source: PathBuf,
    },
    Directory {
        identity: DraftEntryIdentity,
        parent_id: EntryId,
        name: EntryName,
    },
    Remove {
        entry_id: EntryId,
    },
    /// Reuse an immutable file manifest or submit an already assembled edit.
    Reuse(DraftMutation),
}
#[derive(Serialize, Deserialize)]
struct TreeJobFile {
    request: CreateTreeDraftRequest,
    pages: u64,
}
/// A sealed on-disk plan. Preparing is blocking; run large preparations on a blocking thread.
pub struct TreeJob {
    directory: PathBuf,
    saved: TreeJobFile,
}
impl TreeJob {
    /// Consume bounded pages (1..=1000 edits each), preserving their order on disk.
    /// The directory must not exist. No publication occurs during preparation.
    /// A failed preparation has no job.json and cannot accidentally be resumed.
    pub fn prepare(
        directory: &Path,
        request: CreateTreeDraftRequest,
        pages: impl IntoIterator<Item = Result<Vec<TreeMutation>, Error>>,
    ) -> Result<Self, Error> {
        fs::create_dir(directory)?;
        fs::create_dir(directory.join("mutations"))?;
        fs::create_dir(directory.join("files"))?;
        fs::create_dir(directory.join("missing"))?;
        let mut count = 0u64;
        for page in pages {
            let page = page?;
            if page.is_empty() || page.len() > MAX_PAGE_SIZE as usize {
                return Err(Error::Integrity);
            }
            let mut mutations = Vec::with_capacity(page.len());
            for edit in page {
                mutations.push(match edit {
                    TreeMutation::Remove { entry_id } => DraftMutation::Remove { entry_id },
                    TreeMutation::Reuse(edit) => edit,
                    TreeMutation::Directory {
                        identity,
                        parent_id,
                        name,
                    } => DraftMutation::Upsert {
                        identity,
                        parent_id,
                        name,
                        entry_kind: EntryKind::Directory,
                        file: None,
                    },
                    TreeMutation::File {
                        identity,
                        parent_id,
                        name,
                        source,
                    } => {
                        let entry = match &identity {
                            DraftEntryIdentity::Existing { entry_id }
                            | DraftEntryIdentity::New { entry_id } => entry_id,
                        };
                        let local = directory
                            .join("files")
                            .join(digest(entry.as_str().as_bytes()));
                        // One source per identity prevents obsolete, unreachable payload uploads.
                        let file = UploadJob::prepare(
                            &source,
                            &local,
                            FileTarget::Replace {
                                entry_id: entry.clone(),
                                expected_revision: request.base.expected_revision.clone(),
                            },
                            request.expected_policy_revision.clone(),
                            request.idempotency_key.clone(),
                        )?;
                        DraftMutation::Upsert {
                            identity,
                            parent_id,
                            name,
                            entry_kind: EntryKind::File,
                            file: Some(file.manifest().clone()),
                        }
                    }
                });
            }
            let key = digest(format!("{}:{count}", request.idempotency_key.as_str()).as_bytes())
                .parse()
                .map_err(invalid)?;
            let page = DraftMutationPage {
                idempotency_key: key,
                mutations,
            };
            page.validate().map_err(invalid)?;
            durable(
                &directory.join("mutations").join(count.to_string()),
                &canonical(&page).map_err(invalid)?,
            )?;
            count += 1;
        }
        File::open(directory.join("mutations"))?.sync_all()?;
        File::open(directory.join("files"))?.sync_all()?;
        let saved = TreeJobFile {
            request,
            pages: count,
        };
        durable(&directory.join("job.json"), &serde_json::to_vec(&saved)?)?;
        File::open(directory)?.sync_all()?;
        Ok(Self {
            directory: directory.into(),
            saved,
        })
    }
    pub fn open(directory: &Path) -> Result<Self, Error> {
        let saved: TreeJobFile = serde_json::from_reader(File::open(directory.join("job.json"))?)?;
        Ok(Self {
            directory: directory.into(),
            saved,
        })
    }
    pub fn request(&self) -> &CreateTreeDraftRequest {
        &self.saved.request
    }
}
fn durable(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut f = File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
impl Client {
    /// Replay a sealed plan after a disconnect. Cancellation only drops this
    /// request; abort_tree_draft is a separate explicit operation.
    pub async fn upload_tree(&self, job: &TreeJob) -> Result<CommitReceipt, Error> {
        let root = &job.saved.request.base.root_id;
        let mut status = self.create_tree_draft(root, job.request()).await?;
        let draft = status.id.clone();
        if let Some(receipt) = status.result {
            return Ok(receipt);
        }
        while status.phase == TreeDraftPhase::Initializing {
            status = self.advance_tree_draft(root, &draft).await?;
        }
        if status.phase == TreeDraftPhase::Editing {
            for index in 0..job.saved.pages {
                let bytes = fs::read(job.directory.join("mutations").join(index.to_string()))?;
                if bytes.len() > MANIFEST_BYTES {
                    return Err(Error::Integrity);
                }
                let page: DraftMutationPage = serde_json::from_slice(&bytes)?;
                self.mutate_tree_draft(root, &draft, &page).await?;
            }
            for entry in fs::read_dir(job.directory.join("files"))? {
                let file = UploadJob::open(&entry?.path())?;
                for entry in fs::read_dir(file.directory.join("pages"))? {
                    let bytes = fs::read(entry?.path())?;
                    if bytes.len() > MANIFEST_BYTES {
                        return Err(Error::Integrity);
                    }
                    self.tree_draft_page(root, &draft, &serde_json::from_slice(&bytes)?)
                        .await?;
                }
            }
            status = self.freeze_tree_draft(root, &draft).await?;
        }
        while status.admission != AdmissionState::Admitted {
            if !matches!(status.state, SessionState::Open | SessionState::Ready) {
                return Err(Error::Local("tree draft is terminal".into()));
            }
            status = self.admit_tree_draft(root, &draft).await?;
            if status.admission != AdmissionState::Admitted {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        let upload: UploadId = draft.as_str().parse().map_err(invalid)?;
        let missing = job.directory.join("missing");
        let mut cursor = None;
        loop {
            let page = self.missing_chunks(root, &upload, cursor.as_ref()).await?;
            for chunk in page.chunks {
                File::create(missing.join(chunk.digest.as_str()))?;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        for entry in fs::read_dir(job.directory.join("files"))? {
            let source = UploadJob::open(&entry?.path())?;
            self.upload_verified_source(root, &upload, &source, &missing)
                .await?;
        }
        while self.seal_upload(root, &upload).await?.upload.state != SessionState::Ready {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        self.commit_tree_draft(root, &draft).await
    }
    /// Stream a descendant of an immutable directory version into a temporary output.
    pub async fn download_snapshot_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        root: &RootId,
        version: &VersionId,
        path: &RootPath,
        range: Option<(u64, u64)>,
        output: &mut W,
    ) -> Result<u64, Error> {
        use tokio::io::AsyncWriteExt;
        let manifest = self.snapshot_manifest(root, version, path).await?;
        let (start, end) = range.unwrap_or((0, manifest.length.get()));
        if start > end || end > manifest.length.get() || range.is_some_and(|(a, b)| a == b) {
            return Err(Error::Integrity);
        }
        let mut request = self
            .raw(
                Method::GET,
                &format!(
                    "/api/v1/roots/{}/snapshots/{}/content",
                    root.as_str(),
                    version.as_str()
                ),
            )
            .query(&[("path", path.as_str())]);
        if range.is_some() {
            request = request.header("range", format!("bytes={start}-{}", end - 1))
        }
        let mut response =
            crate::transfer::checked(request.send().await.map_err(Error::Transport)?).await?;
        if response.status().as_u16() != if range.is_some() { 206 } else { 200 }
            || response.content_length() != Some(end - start)
        {
            return Err(Error::Integrity);
        }
        if range.is_some()
            && response
                .headers()
                .get("content-range")
                .and_then(|h| h.to_str().ok())
                != Some(format!("bytes {start}-{}/{}", end - 1, manifest.length.get()).as_str())
        {
            return Err(Error::Integrity);
        }
        let mut received = 0;
        while let Some(bytes) = response.chunk().await.map_err(Error::Transport)? {
            received += bytes.len() as u64;
            if received > end - start {
                return Err(Error::Integrity);
            }
            output.write_all(&bytes).await?;
        }
        if received != end - start {
            return Err(Error::Integrity);
        }
        output.flush().await?;
        Ok(received)
    }
}
