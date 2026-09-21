use crate::{
    auth::{Actor, authorize},
    content_catalog::children,
    error::{Error, Result, require},
    payload::{ObjectKey, Reader},
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use sqlx::Row;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncReadExt, time::Instant};

pub struct Download {
    engine: TransferEngine,
    actor: Actor,
    root: RootId,
    pin: String,
    stack: Vec<(String, u64)>,
    policy: ReadPolicy,
    candidate: usize,
    pub length: u64,
    pub start: u64,
    pub end: u64,
    deadline: Option<Instant>,
    alive: Arc<AtomicBool>,
    fetching: Arc<AtomicBool>,
    keepalive: tokio::task::JoinHandle<()>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl Drop for Download {
    fn drop(&mut self) {
        self.keepalive.abort();
        let writer = self.engine.writer.clone();
        let actor = self.actor.clone();
        let pin = self.pin.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = writer.release_read_pin(&actor, &pin).await;
            });
        }
    }
}
impl TransferEngine {
    pub async fn file_manifest(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
    ) -> Result<FileManifest> {
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let r=sqlx::query("SELECT o.body,o.digest FROM file_versions v JOIN content_objects o ON o.root_id=v.root_id AND o.digest=v.manifest WHERE v.id=$1 AND v.root_id=$2 AND v.kind='file' AND NOT v.retired AND NOT entry_purged(v.entry_id)").bind(version.as_str()).bind(root.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        let bytes: Vec<u8> = r.get("body");
        require(
            digest(&bytes) == r.get::<String, _>("digest"),
            ErrorCode::IntegrityFailure,
        )?;
        let manifest: FileManifest = parse_canonical(&bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub async fn download(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        range: Option<(u64, u64)>,
    ) -> Result<Download> {
        self.download_inner(actor, root, version, None, range).await
    }
    pub async fn snapshot_file_manifest(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        path: &RootPath,
    ) -> Result<FileManifest> {
        let parts = crate::tree_catalog::path_parts(path)?;
        let entry = self
            .writer
            .snapshot_entry(actor, root, version, &parts)
            .await?;
        require(entry.kind == EntryKind::File, ErrorCode::InvalidRequest)?;
        let mut tx = self.writer.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let manifest: FileManifest =
            crate::tree_catalog::metadata(&mut tx, root.as_str(), entry.manifest.as_str(), "file")
                .await?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub async fn download_snapshot_file(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        path: &RootPath,
        range: Option<(u64, u64)>,
    ) -> Result<Download> {
        let parts = crate::tree_catalog::path_parts(path)?;
        require(!parts.is_empty(), ErrorCode::InvalidRequest)?;
        self.download_inner(actor, root, version, Some(&parts), range)
            .await
    }
    async fn download_inner(
        &self,
        actor: &Actor,
        root: &RootId,
        version: &VersionId,
        path: Option<&[EntryName]>,
        range: Option<(u64, u64)>,
    ) -> Result<Download> {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
        let pin = self.writer.pin_version(actor, root, version).await?;
        let setup=async{
            let mut tx=self.writer.transaction().await?;authorize(&mut tx,actor,root.as_str(),Capability::ListRead).await?;
            // Pin acquisition already fixed this identity, even if rotation has
            // since retired it. No new pin can be acquired after that retirement.
            let row=sqlx::query("SELECT v.kind,v.manifest,r.placement FROM file_versions v JOIN roots r ON r.id=v.root_id WHERE v.id=$1 AND v.root_id=$2").bind(version.as_str()).bind(root.as_str()).fetch_one(&mut *tx).await?;
            let manifest:String=row.get("manifest");
            let manifest=if let Some(path)=path {
                require(row.get::<String,_>("kind")=="directory",ErrorCode::NotFound)?;
                let entry=crate::tree_catalog::resolve(&mut tx,root.as_str(),&manifest,path).await?;
                require(entry.kind==EntryKind::File,ErrorCode::InvalidRequest)?;
                entry.manifest.as_str().to_owned()
            } else { require(row.get::<String,_>("kind")=="file",ErrorCode::NotFound)?;manifest };
            let manifest:FileManifest=crate::tree_catalog::metadata(&mut tx,root.as_str(),&manifest,"file").await?;manifest.validate()?;
            let placement:Placement=serde_json::from_value(row.get("placement"))?;
            let policy=match placement{Placement::Single{read_policy,..}|Placement::Mirror{read_policy,..}=>read_policy};
            let length=manifest.length.get();let(start,end)=range.unwrap_or((0,length));
            require(start<=end&&end<=length&&(range.is_none()||start<end),ErrorCode::InvalidRequest)?;
            Ok::<_,Error>((manifest,policy,start,end))
        }.await;
        let (manifest, policy, start, end) = match setup {
            Ok(v) => v,
            Err(e) => {
                self.writer.release_read_pin(actor, &pin).await?;
                return Err(e);
            }
        };
        let alive = Arc::new(AtomicBool::new(true));
        let fetching = Arc::new(AtomicBool::new(false));
        let renew_writer = self.writer.clone();
        let renew_actor = actor.clone();
        let renew_root = root.clone();
        let renew_pin = pin.clone();
        let renew_alive = alive.clone();
        let renew_fetching = fetching.clone();
        let keepalive = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if renew_fetching.load(Ordering::Acquire)
                    && renew_writer
                        .renew_read_pin(&renew_actor, &renew_root, &renew_pin)
                        .await
                        .is_err()
                {
                    renew_alive.store(false, Ordering::Release);
                    break;
                }
            }
        });
        Ok(Download {
            engine: self.clone(),
            actor: actor.clone(),
            root: root.clone(),
            pin,
            stack: manifest
                .tree
                .map(|t| vec![(t.as_str().into(), 0)])
                .unwrap_or_default(),
            candidate: 0,
            length: manifest.length.get(),
            start,
            end,
            deadline: policy
                .overall_deadline_ms
                .map(|d| Instant::now() + Duration::from_millis(d.get())),
            policy,
            alive,
            fetching,
            keepalive,
            _permit: permit,
        })
    }
}
impl Download {
    /// Returns only a verified chunk (or its verified range intersection).
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>> {
        require(
            self.alive.load(Ordering::Acquire),
            ErrorCode::DestinationUnavailable,
        )?;
        self.engine
            .writer
            .renew_read_pin(&self.actor, &self.root, &self.pin)
            .await?;
        self.fetching.store(true, Ordering::Release);
        let result = self.next_inner().await;
        self.fetching.store(false, Ordering::Release);
        require(
            self.alive.load(Ordering::Acquire),
            ErrorCode::DestinationUnavailable,
        )?;
        result
    }
    async fn next_inner(&mut self) -> Result<Option<Vec<u8>>> {
        while let Some((digest, offset)) = self.stack.pop() {
            if self.deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(Error(ErrorCode::DestinationUnavailable));
            }
            let mut tx = self.engine.writer.transaction().await?;
            let row = sqlx::query(
                "SELECT kind,length FROM content_objects WHERE root_id=$1 AND digest=$2",
            )
            .bind(self.root.as_str())
            .bind(&digest)
            .fetch_one(&mut *tx)
            .await?;
            let length = row.get::<i64, _>("length") as u64;
            if offset >= self.end || offset + length <= self.start {
                continue;
            }
            if row.get::<String, _>("kind") != "chunk" {
                let children = children(&mut tx, self.root.as_str(), &digest).await?;
                let mut position = offset;
                let mut pending = Vec::with_capacity(children.len());
                for (child, length, _) in children {
                    pending.push((child, position));
                    position += length;
                }
                self.stack.extend(pending.into_iter().rev());
                continue;
            }
            drop(tx);
            let mut failure = Error(ErrorCode::DestinationUnavailable);
            while self.candidate < self.policy.candidates.len() {
                let candidate = &self.policy.candidates[self.candidate];
                let result = self.read_chunk(&digest, length, candidate).await;
                match result {
                    Ok(bytes) => {
                        // Check the live pin once more after arbitrarily slow I/O.
                        self.engine
                            .writer
                            .renew_read_pin(&self.actor, &self.root, &self.pin)
                            .await?;
                        let start = self.start.saturating_sub(offset) as usize;
                        let end = (self.end - offset).min(length) as usize;
                        return Ok(Some(bytes[start..end].to_vec()));
                    }
                    Err(e) => {
                        failure = e;
                        self.candidate += 1;
                    }
                }
            }
            return Err(failure);
        }
        Ok(None)
    }
    async fn read_chunk(
        &self,
        digest: &str,
        length: u64,
        candidate: &ReadCandidate,
    ) -> Result<Vec<u8>> {
        let mut tx = self.engine.writer.transaction().await?;
        let generation:Option<String>=sqlx::query_scalar("SELECT generation FROM object_copies WHERE root_id=$1 AND digest=$2 AND backend_id=$3 AND state='available'").bind(self.root.as_str()).bind(digest).bind(candidate.backend_id.as_str()).fetch_optional(&mut *tx).await?;
        drop(tx);
        let key = ObjectKey {
            root: self.root.clone(),
            digest: digest.parse()?,
            generation: generation
                .ok_or(Error(ErrorCode::DestinationUnavailable))?
                .parse()?,
        };
        let store = self.engine.store(candidate.backend_id.as_str())?;
        let initial = Duration::from_millis(
            candidate.connection_timeout_ms.get() + candidate.initial_response_timeout_ms.get(),
        );
        let initial = self
            .deadline
            .map(|d| initial.min(d.saturating_duration_since(Instant::now())))
            .unwrap_or(initial);
        let reader = tokio::time::timeout(initial, store.read_with_policy(&key, candidate))
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))??;
        let bytes = read_progress(reader, length, candidate, self.deadline).await?;
        require(
            bytes.len() as u64 == length && sct_protocol::digest(&bytes) == digest,
            ErrorCode::IntegrityFailure,
        )?;
        Ok(bytes)
    }
}
pub(crate) async fn read_progress(
    mut reader: Reader,
    length: u64,
    candidate: &ReadCandidate,
    deadline: Option<Instant>,
) -> Result<Vec<u8>> {
    require(length <= u64::from(CHUNK_MAX), ErrorCode::IntegrityFailure)?;
    let mut bytes = Vec::with_capacity(length as usize);
    let mut buffer = [0u8; 16384];
    let mut last = Instant::now();
    let mut window = last + Duration::from_millis(candidate.startup_allowance_ms.get());
    let mut window_bytes = 0u64;
    let window_length = Duration::from_millis(candidate.throughput_window_ms.get());
    loop {
        let now = Instant::now();
        if now >= window + window_length {
            require(
                u128::from(window_bytes) * 1000
                    >= u128::from(candidate.minimum_bytes_per_second.get())
                        * now.duration_since(window).as_millis(),
                ErrorCode::DestinationUnavailable,
            )?;
            window = now;
            window_bytes = 0;
        }
        let mut until = (last + Duration::from_millis(candidate.idle_timeout_ms.get()))
            .min(window + window_length);
        if let Some(deadline) = deadline {
            until = until.min(deadline);
        }
        let result = tokio::time::timeout_at(until, reader.read(&mut buffer)).await;
        match result {
            Ok(Ok(0)) => return Ok(bytes),
            Ok(Ok(n)) => {
                last = Instant::now();
                window_bytes += n as u64;
                require(
                    bytes.len() + n <= length as usize,
                    ErrorCode::IntegrityFailure,
                )?;
                bytes.extend_from_slice(&buffer[..n]);
            }
            Ok(Err(_)) => return Err(Error(ErrorCode::DestinationUnavailable)),
            Err(_) => {
                let now = Instant::now();
                require(
                    now < last + Duration::from_millis(candidate.idle_timeout_ms.get())
                        && deadline.is_none_or(|d| now < d),
                    ErrorCode::DestinationUnavailable,
                )?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn progress_budget_accepts_slow_progress_but_rejects_stalls() {
        let candidate: ReadCandidate = serde_json::from_value(serde_json::json!({"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":100,"throughput_window_ms":50,"minimum_bytes_per_second":1,"startup_allowance_ms":1})).unwrap();
        let (reader, mut writer) = tokio::io::duplex(16);
        let producer = tokio::spawn(async move {
            for _ in 0..6 {
                writer.write_all(b"x").await.unwrap();
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        });
        assert_eq!(
            read_progress(Box::pin(reader), 6, &candidate, None)
                .await
                .unwrap(),
            b"xxxxxx"
        );
        producer.await.unwrap();
        let (reader, _writer) = tokio::io::duplex(16);
        assert!(
            read_progress(Box::pin(reader), 6, &candidate, None)
                .await
                .is_err()
        );
    }
}
