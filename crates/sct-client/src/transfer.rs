//! Durable bounded-memory upload jobs. The caller owns source capture and job cleanup.
use crate::{Client, Error};
use sct_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct JobFile {
    source: PathBuf,
    request: CreateUploadRequest,
}
pub struct UploadJob {
    directory: PathBuf,
    saved: JobFile,
}
impl UploadJob {
    /// Capture a plan for a caller-owned stable file. `directory` must not exist.
    /// Run on a blocking thread for large sources. Never point this at a live DB.
    pub fn prepare(
        source: &Path,
        directory: &Path,
        target: FileTarget,
        policy: Revision,
        key: IdempotencyKey,
    ) -> Result<Self, Error> {
        fs::create_dir(directory)?;
        fs::create_dir(directory.join("pages"))?;
        fs::create_dir(directory.join("missing"))?;
        let source = fs::canonicalize(source)?;
        let mut index = BufWriter::new(File::create(directory.join("chunks.jsonl"))?);
        let mut refs = BufWriter::new(File::create(directory.join("level-0.jsonl"))?);
        let mut leaf = Vec::new();
        let mut length = 0u64;
        let mut count = 0u64;
        for chunk in chunker(File::open(&source)?) {
            let chunk = chunk.map_err(|e| Error::Local(e.to_string()))?;
            let reference = ChunkReference {
                digest: digest(&chunk.data).parse().map_err(invalid)?,
                length: (chunk.data.len() as u64).try_into().map_err(invalid)?,
            };
            line(&mut index, &reference)?;
            length = length
                .checked_add(reference.length.get())
                .ok_or(Error::Integrity)?;
            count += 1;
            leaf.push(reference);
            if leaf.len() == MANIFEST_FANOUT {
                let r = page(
                    directory,
                    ManifestPage::Chunks {
                        format: 1,
                        chunks: std::mem::take(&mut leaf),
                    },
                )?;
                line(&mut refs, &r)?;
            }
        }
        if !leaf.is_empty() {
            line(
                &mut refs,
                &page(
                    directory,
                    ManifestPage::Chunks {
                        format: 1,
                        chunks: leaf,
                    },
                )?,
            )?;
        }
        index.flush()?;
        index.get_ref().sync_all()?;
        refs.flush()?;
        drop(refs);
        let mut height = 0;
        let tree = if count == 0 {
            None
        } else {
            loop {
                let input =
                    BufReader::new(File::open(directory.join(format!("level-{height}.jsonl")))?);
                let mut items = input.lines();
                let first: PageReference =
                    serde_json::from_str(&items.next().ok_or(Error::Integrity)??)?;
                let Some(second) = items.next() else {
                    break Some(first.digest);
                };
                if height >= MANIFEST_HEIGHT {
                    return Err(Error::Integrity);
                }
                let mut output = BufWriter::new(File::create(
                    directory.join(format!("level-{}.jsonl", height + 1)),
                )?);
                let mut group = vec![first, serde_json::from_str(&second?)?];
                for item in items {
                    group.push(serde_json::from_str(&item?)?);
                    if group.len() == MANIFEST_FANOUT {
                        line(
                            &mut output,
                            &page(
                                directory,
                                ManifestPage::Branch {
                                    format: 1,
                                    height: height + 1,
                                    pages: std::mem::take(&mut group),
                                },
                            )?,
                        )?;
                    }
                }
                if !group.is_empty() {
                    line(
                        &mut output,
                        &page(
                            directory,
                            ManifestPage::Branch {
                                format: 1,
                                height: height + 1,
                                pages: group,
                            },
                        )?,
                    )?;
                }
                output.flush()?;
                height += 1;
            }
        };
        let manifest = FileManifest {
            format: 1,
            profile: CHUNK_PROFILE.into(),
            length: length.try_into().map_err(invalid)?,
            chunks: count.try_into().map_err(invalid)?,
            height,
            tree,
        };
        manifest.validate().map_err(invalid)?;
        let saved = JobFile {
            source,
            request: CreateUploadRequest {
                idempotency_key: key,
                target,
                expected_policy_revision: policy,
                manifest,
            },
        };
        File::open(directory.join("pages"))?.sync_all()?;
        let mut file = File::create(directory.join("job.json"))?;
        file.write_all(&serde_json::to_vec(&saved)?)?;
        file.sync_all()?;
        File::open(directory)?.sync_all()?;
        Ok(Self {
            directory: directory.into(),
            saved,
        })
    }
    pub fn open(directory: &Path) -> Result<Self, Error> {
        let saved: JobFile = serde_json::from_reader(File::open(directory.join("job.json"))?)?;
        saved.request.manifest.validate().map_err(invalid)?;
        Ok(Self {
            directory: directory.into(),
            saved,
        })
    }
    pub fn request(&self) -> &CreateUploadRequest {
        &self.saved.request
    }
    pub fn manifest(&self) -> &FileManifest {
        &self.saved.request.manifest
    }
}
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Local(e.to_string())
}
fn line(w: &mut impl Write, value: &impl Serialize) -> Result<(), Error> {
    serde_json::to_writer(&mut *w, value)?;
    w.write_all(b"\n")?;
    Ok(())
}
fn page(directory: &Path, page: ManifestPage) -> Result<PageReference, Error> {
    page.validate().map_err(invalid)?;
    let (length, chunks) = match &page {
        ManifestPage::Chunks { chunks, .. } => (
            chunks.iter().map(|c| c.length.get()).sum::<u64>(),
            chunks.len() as u64,
        ),
        ManifestPage::Branch { pages, .. } => (
            pages.iter().map(|p| p.length.get()).sum(),
            pages.iter().map(|p| p.chunks.get()).sum(),
        ),
    };
    let bytes = canonical(&page).map_err(invalid)?;
    let hash = digest(&bytes);
    let mut f = File::create(directory.join("pages").join(&hash))?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    Ok(PageReference {
        digest: hash.parse().map_err(invalid)?,
        length: length.try_into().map_err(invalid)?,
        chunks: chunks.try_into().map_err(invalid)?,
    })
}
fn path(root: &RootId, upload: &UploadId) -> String {
    format!(
        "/api/v1/roots/{}/uploads/{}",
        root.as_str(),
        upload.as_str()
    )
}
impl Client {
    pub async fn create_upload(
        &self,
        root: &RootId,
        request: &CreateUploadRequest,
    ) -> Result<TransferStatus, Error> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/roots/{}/uploads", root.as_str()),
            vec![],
            Some(serde_json::to_value(request)?),
        )
        .await
    }
    pub async fn upload_status(
        &self,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus, Error> {
        self.request(reqwest::Method::GET, &path(root, upload), vec![], None)
            .await
    }
    pub async fn abort_upload(
        &self,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus, Error> {
        self.action(root, upload, "abort").await
    }
    pub async fn admit_upload(
        &self,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus, Error> {
        self.action(root, upload, "admit").await
    }
    pub async fn seal_upload(
        &self,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<TransferStatus, Error> {
        self.action(root, upload, "seal").await
    }
    pub async fn commit_upload(
        &self,
        root: &RootId,
        upload: &UploadId,
    ) -> Result<CommitReceipt, Error> {
        self.action(root, upload, "commit").await
    }
    async fn action<T: serde::de::DeserializeOwned>(
        &self,
        root: &RootId,
        upload: &UploadId,
        action: &str,
    ) -> Result<T, Error> {
        self.request(
            reqwest::Method::POST,
            &format!("{}/{action}", path(root, upload)),
            vec![],
            None,
        )
        .await
    }
    pub async fn upload_page(
        &self,
        root: &RootId,
        upload: &UploadId,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        let hash = digest(&bytes);
        self.put_bytes(&format!("{}/pages/{hash}", path(root, upload)), bytes)
            .await
    }
    pub async fn upload_chunk(
        &self,
        root: &RootId,
        upload: &UploadId,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        let hash = digest(&bytes);
        self.put_bytes(&format!("{}/chunks/{hash}", path(root, upload)), bytes)
            .await
    }
    async fn put_bytes(&self, path: &str, bytes: Vec<u8>) -> Result<(), Error> {
        let response = self
            .raw(reqwest::Method::PUT, path)
            .body(bytes)
            .send()
            .await
            .map_err(Error::Transport)?;
        checked(response).await?;
        Ok(())
    }
    fn raw(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut url = self.origin.clone();
        url.set_path(path);
        let request = self.http.request(method, url);
        if let Some(token) = &self.token {
            request.bearer_auth(token)
        } else {
            request
        }
    }
    pub async fn missing_chunks(
        &self,
        root: &RootId,
        upload: &UploadId,
        cursor: Option<&CursorToken>,
    ) -> Result<MissingChunkPage, Error> {
        self.request(
            reqwest::Method::GET,
            &format!("{}/chunks", path(root, upload)),
            cursor
                .map(|c| vec![("cursor", c.as_str().into())])
                .unwrap_or_default(),
            None,
        )
        .await
    }
    pub async fn file_history(
        &self,
        root: &RootId,
        entry: &EntryId,
        cursor: Option<&CursorToken>,
    ) -> Result<VersionPage, Error> {
        self.request(
            reqwest::Method::GET,
            &format!(
                "/api/v1/roots/{}/entries/{}/versions",
                root.as_str(),
                entry.as_str()
            ),
            cursor
                .map(|c| vec![("cursor", c.as_str().into())])
                .unwrap_or_default(),
            None,
        )
        .await
    }
    pub async fn file_manifest(
        &self,
        root: &RootId,
        version: &VersionId,
    ) -> Result<FileManifest, Error> {
        self.request(
            reqwest::Method::GET,
            &format!(
                "/api/v1/roots/{}/versions/{}/manifest",
                root.as_str(),
                version.as_str()
            ),
            vec![],
            None,
        )
        .await
    }
    /// Resume by replaying the durable job's idempotency key. Drop this future to
    /// disconnect; call again with UploadJob::open to resume. Abort is explicit.
    pub async fn upload(&self, root: &RootId, job: &UploadJob) -> Result<CommitReceipt, Error> {
        let mut status = self.create_upload(root, job.request()).await?;
        let upload = status.upload.id.clone();
        if status.upload.state == SessionState::Committed {
            return self.commit_upload(root, &upload).await;
        }
        if !matches!(
            status.upload.state,
            SessionState::Open | SessionState::Ready
        ) {
            return Err(Error::Local("session is terminal".into()));
        }
        if status.admission == AdmissionState::Planning {
            for entry in fs::read_dir(job.directory.join("pages"))? {
                let entry = entry?;
                let bytes = fs::read(entry.path())?;
                if bytes.len() > MANIFEST_BYTES {
                    return Err(Error::Integrity);
                }
                self.upload_page(root, &upload, bytes).await?;
            }
        }
        while status.admission != AdmissionState::Admitted {
            if status.admission == AdmissionState::Rejected {
                return Err(Error::Local(format!(
                    "admission rejected: {:?}",
                    status.failure
                )));
            }
            status = self.admit_upload(root, &upload).await?;
            if status.admission != AdmissionState::Admitted {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        let mut cursor = None;
        loop {
            let page = self.missing_chunks(root, &upload, cursor.as_ref()).await?;
            for chunk in page.chunks {
                File::create(job.directory.join("missing").join(chunk.digest.as_str()))?;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        // Second sequential pass verifies even reusable chunks, not just missing ones.
        let mut index = BufReader::new(File::open(job.directory.join("chunks.jsonl"))?).lines();
        for chunk in chunker(File::open(&job.saved.source)?) {
            let chunk = chunk.map_err(|e| Error::Local(e.to_string()))?;
            let expected: ChunkReference =
                serde_json::from_str(&index.next().ok_or(Error::SourceChanged)??)?;
            if expected.length.get() != chunk.data.len() as u64
                || expected.digest.as_str() != digest(&chunk.data)
            {
                return Err(Error::SourceChanged);
            }
            if job
                .directory
                .join("missing")
                .join(expected.digest.as_str())
                .exists()
            {
                self.upload_chunk(root, &upload, chunk.data).await?;
            }
        }
        if index.next().is_some() {
            return Err(Error::SourceChanged);
        }
        while self.seal_upload(root, &upload).await?.upload.state != SessionState::Ready {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // Lost receipts are recovered by replaying this same job/commit.
        self.commit_upload(root, &upload).await
    }
    /// Stream to a caller-owned temporary output; promote it only after success.
    /// Range is [start,end), matching Rust slice conventions.
    pub async fn download_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        root: &RootId,
        version: &VersionId,
        range: Option<(u64, u64)>,
        output: &mut W,
    ) -> Result<u64, Error> {
        use tokio::io::AsyncWriteExt;
        let manifest = self.file_manifest(root, version).await?;
        let (start, end) = range.unwrap_or((0, manifest.length.get()));
        if start > end || end > manifest.length.get() || range.is_some_and(|(a, b)| a == b) {
            return Err(Error::Integrity);
        }
        let mut request = self.raw(
            reqwest::Method::GET,
            &format!(
                "/api/v1/roots/{}/versions/{}/content",
                root.as_str(),
                version.as_str()
            ),
        );
        if range.is_some() {
            request = request.header("range", format!("bytes={start}-{}", end - 1));
        }
        let mut response = checked(request.send().await.map_err(Error::Transport)?).await?;
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
        let mut received = 0u64;
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
async fn checked(response: reqwest::Response) -> Result<reqwest::Response, Error> {
    if response.status().is_success() {
        Ok(response)
    } else {
        let status = response.status().as_u16();
        Err(match response.json::<ErrorResponse>().await {
            Ok(e) => Error::Server(Box::new(e)),
            Err(_) => Error::UnexpectedStatus(status),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_plan_roundtrips_empty_and_multichunk_sources() {
        let base = std::env::temp_dir().join(format!(
            "sct-sdk-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&base).unwrap();
        for length in [0, 12 * 1024 * 1024] {
            let source = base.join(format!("source-{length}"));
            let mut state = 0x9e3779b97f4a7c15u64;
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect();
            fs::write(&source, &bytes).unwrap();
            let directory = base.join(format!("job-{length}"));
            let job = UploadJob::prepare(
                &source,
                &directory,
                FileTarget::Create {
                    parent_id: "parent".parse().unwrap(),
                    expected_parent_revision: "revision".parse().unwrap(),
                    name: "file".parse().unwrap(),
                },
                "policy".parse().unwrap(),
                "key".parse().unwrap(),
            )
            .unwrap();
            assert_eq!(job.manifest().length.get(), length as u64);
            let reopened = UploadJob::open(&directory).unwrap();
            assert_eq!(job.request(), reopened.request());
            let mut total = 0;
            for line in BufReader::new(File::open(directory.join("chunks.jsonl")).unwrap()).lines()
            {
                let c: ChunkReference = serde_json::from_str(&line.unwrap()).unwrap();
                let end = total + c.length.get() as usize;
                assert_eq!(c.digest.as_str(), digest(&bytes[total..end]));
                total = end;
            }
            assert_eq!(total, length);
            for entry in fs::read_dir(directory.join("pages")).unwrap() {
                let e = entry.unwrap();
                let bytes = fs::read(e.path()).unwrap();
                let page: ManifestPage = parse_canonical(&bytes).unwrap();
                page.validate().unwrap();
                assert_eq!(e.file_name().to_str().unwrap(), digest(&bytes));
            }
        }
        fs::remove_dir_all(base).unwrap();
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[tokio::test]
    async fn truncated_http_body_never_reports_success() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let manifest=serde_json::json!({"format":1,"profile":CHUNK_PROFILE,"length":6,"chunks":1,"height":1,"tree":"a".repeat(64)}).to_string();
            for body in [manifest.as_bytes(), b"abc"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                let length = if body == b"abc" { 6 } else { body.len() };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        let client = Client::new(origin.parse().unwrap()).unwrap();
        let mut output = Vec::new();
        assert!(
            client
                .download_to(
                    &"root".parse().unwrap(),
                    &"version".parse().unwrap(),
                    None,
                    &mut output
                )
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}
