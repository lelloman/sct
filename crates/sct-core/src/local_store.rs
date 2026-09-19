//! Local immutable storage. Requires a private directory on a filesystem honoring
//! hard-link/create-new and file/directory fsync; no cross-device temporary files.
use crate::{
    backend::Adapter,
    error::{Error, Result, require},
    payload::*,
};
use sct_protocol::{AdapterDocumentation, ErrorCode, ObservedHealth};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    directory: PathBuf,
    #[serde(default)]
    headroom_bytes: u64,
}
pub struct LocalAdapter;
pub struct LocalStore {
    directory: PathBuf,
    headroom: u64,
}
fn io(e: std::io::Error) -> Error {
    Error(match e.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::StorageFull => ErrorCode::DestinationCapacity,
        _ => ErrorCode::DestinationUnavailable,
    })
}
impl LocalStore {
    pub fn new(directory: &Path, headroom: u64) -> Result<Self> {
        let directory = directory.canonicalize().map_err(io)?;
        require(directory.is_dir(), ErrorCode::InvalidRequest)?;
        Ok(Self {
            directory,
            headroom,
        })
    }
    fn path(&self, key: &ObjectKey) -> PathBuf {
        self.directory.join(key.path())
    }
}
impl Adapter for LocalAdapter {
    fn documentation(&self) -> AdapterDocumentation {
        AdapterDocumentation {
            kind: "local".into(),
            display_name: "Local filesystem".into(),
            acknowledgement: "Verified bytes, fsync file, no-replace hard link, fsync directories"
                .into(),
            consistency: "Local filesystem read-after-write".into(),
            durability: "Depends on filesystem and hardware honoring fsync".into(),
            deletion_and_capacity:
                "Unlink and fsync; statvfs available bytes minus configured headroom".into(),
            limitations: vec![
                "Private service-owned directory; no external writers or symlink substitution"
                    .into(),
                "Temporary files must be on the destination filesystem".into(),
            ],
        }
    }
    fn validate(&self, settings: &serde_json::Value) -> Result<String> {
        let s: Settings = serde_json::from_value(settings.clone())?;
        let store = LocalStore::new(&s.directory, s.headroom_bytes)?;
        Ok(format!("local:{}", store.directory.display()))
    }
    fn probe(&self, settings: &serde_json::Value) -> ObservedHealth {
        if self.validate(settings).is_ok() {
            ObservedHealth::Reachable
        } else {
            ObservedHealth::Unavailable
        }
    }
    fn open(&self, settings: &serde_json::Value) -> Result<Arc<dyn ObjectStore>> {
        let s: Settings = serde_json::from_value(settings.clone())?;
        Ok(Arc::new(LocalStore::new(&s.directory, s.headroom_bytes)?))
    }
}
fn sync_directory(path: &Path) -> Result<()> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io)
}
fn read_existing(path: &Path, key: &ObjectKey, length: u64) -> Result<()> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(io)?
        .take(length + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    require(
        bytes.len() as u64 == length && sct_protocol::digest(&bytes) == key.digest.as_str(),
        ErrorCode::IntegrityFailure,
    )
}
impl ObjectStore for LocalStore {
    fn put<'a>(&'a self, key: &'a ObjectKey, length: u64, body: Reader) -> IoFuture<'a, ()> {
        Box::pin(async move {
            let bytes = verified(body, key, length).await?;
            if self.stat(key).await?.is_some() {
                verified(self.read(key).await?, key, length).await?;
                return Ok(());
            }
            require(
                self.available().await?.is_some_and(|v| v >= length),
                ErrorCode::DestinationCapacity,
            )?;
            let path = self.path(key);
            let base = self.directory.clone();
            let key = key.clone();
            tokio::task::spawn_blocking(move || {
                if path.exists() {
                    return read_existing(&path, &key, length);
                }
                let parent = path.parent().unwrap();
                std::fs::create_dir_all(parent).map_err(io)?;
                let temp = path.with_extension("pending");
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)
                    .map_err(io)?;
                file.write_all(&bytes).map_err(io)?;
                file.sync_all().map_err(io)?;
                match std::fs::hard_link(&temp, &path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        read_existing(&path, &key, length)?
                    }
                    Err(e) => return Err(io(e)),
                }
                std::fs::remove_file(&temp).map_err(io)?;
                let mut directory = parent;
                loop {
                    sync_directory(directory)?;
                    if directory == base {
                        break;
                    }
                    directory = directory.parent().ok_or(Error(ErrorCode::InternalError))?;
                }
                Ok(())
            })
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?
        })
    }
    fn read<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Reader> {
        Box::pin(async move {
            Ok(Box::pin(tokio::fs::File::open(self.path(key)).await.map_err(io)?) as Reader)
        })
    }
    fn stat<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Option<u64>> {
        Box::pin(async move {
            match tokio::fs::metadata(self.path(key)).await {
                Ok(m) => Ok(Some(m.len())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io(e)),
            }
        })
    }
    fn delete<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, DeleteOutcome> {
        Box::pin(async move {
            let path = self.path(key);
            tokio::task::spawn_blocking(move || {
                for p in [&path, &path.with_extension("pending")] {
                    match std::fs::remove_file(p) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(io(e)),
                    }
                }
                if let Some(parent) = path.parent().filter(|p| p.exists()) {
                    sync_directory(parent)?;
                }
                Ok(DeleteOutcome::Confirmed)
            })
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?
        })
    }
    fn available(&self) -> IoFuture<'_, Option<u64>> {
        Box::pin(async move {
            let path = self.directory.clone();
            let headroom = self.headroom;
            tokio::task::spawn_blocking(move || {
                let s = rustix::fs::statvfs(&path)
                    .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
                Ok(Some(
                    s.f_bavail
                        .saturating_mul(s.f_frsize)
                        .saturating_sub(headroom),
                ))
            })
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))?
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_filesystem_conformance_and_partial_cleanup() {
        let directory = std::env::temp_dir().join(format!("sct-local-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let store = LocalStore::new(&directory, 0).unwrap();
        crate::payload::tests::conformance(&store).await;
        let key = crate::payload::tests::key(b"retry", "interrupted");
        let path = store.path(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path.with_extension("pending"), b"ret").unwrap();
        assert!(store.put(&key, 5, reader(b"retry".to_vec())).await.is_err());
        store.delete(&key).await.unwrap();
        store.put(&key, 5, reader(b"retry".to_vec())).await.unwrap();
        let reopened = LocalStore::new(&directory, 0).unwrap();
        assert_eq!(
            verified(reopened.read(&key).await.unwrap(), &key, 5)
                .await
                .unwrap(),
            b"retry"
        );
        let full = LocalStore::new(&directory, u64::MAX).unwrap();
        assert_eq!(full.available().await.unwrap(), Some(0));
        assert!(
            full.put(
                &crate::payload::tests::key(b"x", "full"),
                1,
                reader(b"x".to_vec())
            )
            .await
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
