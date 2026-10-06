//! Asynchronous, provider-independent immutable object operations.
use crate::error::{Error, Result, require};
use sct_protocol::{CHUNK_MAX, ChunkDigest, ErrorCode, Revision, RootId, digest};
use simple_server::io::{AsyncRead, AsyncReadExt};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

pub type Reader = Pin<Box<dyn AsyncRead + Send + Unpin>>;
pub type IoFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectKey {
    pub root: RootId,
    pub digest: ChunkDigest,
    pub generation: Revision,
}
impl ObjectKey {
    pub fn path(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.root.as_str(),
            &self.digest.as_str()[..2],
            self.digest.as_str(),
            self.generation.as_str()
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Confirmed,
    Unknown,
}
pub trait ObjectStore: Send + Sync {
    /// Never overwrite a physical key. Existing identical content is retryable.
    fn put<'a>(&'a self, key: &'a ObjectKey, length: u64, body: Reader) -> IoFuture<'a, ()>;
    fn read<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Reader>;
    fn read_with_policy<'a>(
        &'a self,
        key: &'a ObjectKey,
        _policy: &'a sct_protocol::ReadCandidate,
    ) -> IoFuture<'a, Reader> {
        self.read(key)
    }
    fn stat<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Option<u64>>;
    /// Remove payload and retain a zero-byte generation tombstone so late conditional
    /// writes cannot resurrect deleted bytes. Tombstone metadata uses pool overhead.
    fn delete<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, DeleteOutcome>;
    /// Additional physical-space bound; None still requires a catalog budget.
    fn available(&self) -> IoFuture<'_, Option<u64>>;
}
pub type Stores = BTreeMap<String, Arc<dyn ObjectStore>>;
/// At most one immutable chunk is buffered. Read one extra byte to detect a
/// lying Content-Length; never trust provider checksums, stat or ETag for data.
pub async fn verified(mut body: Reader, key: &ObjectKey, length: u64) -> Result<Vec<u8>> {
    require(
        (1..=u64::from(CHUNK_MAX)).contains(&length),
        ErrorCode::InvalidRequest,
    )?;
    let mut bytes = Vec::with_capacity(length as usize);
    (&mut body)
        .take(length + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
    require(
        bytes.len() as u64 == length && digest(&bytes) == key.digest.as_str(),
        ErrorCode::IntegrityFailure,
    )?;
    Ok(bytes)
}
pub fn reader(bytes: Vec<u8>) -> Reader {
    Box::pin(simple_server::io::Cursor::new(bytes))
}

/// Deterministic failure injection. Production configuration never registers this.
#[derive(Clone, Debug, Default)]
pub enum Fault {
    #[default]
    None,
    Offline,
    LostWriteResponse,
    CorruptRead,
    TruncateRead,
    Delay(std::time::Duration),
    UnknownDelete,
}
#[derive(Default)]
pub struct MemoryStore {
    objects: simple_server::sync::Mutex<BTreeMap<ObjectKey, Vec<u8>>>,
    fault: simple_server::sync::Mutex<Fault>,
}
impl MemoryStore {
    pub async fn fault(&self, fault: Fault) {
        *self.fault.lock().await = fault;
    }
    async fn before(&self) -> Result<Fault> {
        let f = self.fault.lock().await.clone();
        match f {
            Fault::Offline => Err(Error(ErrorCode::DestinationUnavailable)),
            Fault::Delay(d) => {
                simple_server::time::sleep(d).await;
                Ok(Fault::None)
            }
            _ => Ok(f),
        }
    }
}
impl ObjectStore for MemoryStore {
    fn put<'a>(&'a self, key: &'a ObjectKey, length: u64, body: Reader) -> IoFuture<'a, ()> {
        Box::pin(async move {
            let f = self.before().await?;
            let bytes = verified(body, key, length).await?;
            let mut objects = self.objects.lock().await;
            if let Some(existing) = objects.get(key) {
                require(*existing == bytes, ErrorCode::IntegrityFailure)?;
            } else {
                objects.insert(key.clone(), bytes);
            }
            require(
                !matches!(f, Fault::LostWriteResponse),
                ErrorCode::DestinationUnavailable,
            )
        })
    }
    fn read<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Reader> {
        Box::pin(async move {
            let f = self.before().await?;
            let mut bytes = self
                .objects
                .lock()
                .await
                .get(key)
                .filter(|v| !v.is_empty())
                .cloned()
                .ok_or(Error(ErrorCode::NotFound))?;
            match f {
                Fault::CorruptRead => {
                    if let Some(first) = bytes.first_mut() {
                        *first ^= 1;
                    }
                }
                Fault::TruncateRead => {
                    bytes.pop();
                }
                _ => {}
            }
            Ok(reader(bytes))
        })
    }
    fn stat<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Option<u64>> {
        Box::pin(async move {
            self.before().await?;
            Ok(self
                .objects
                .lock()
                .await
                .get(key)
                .filter(|v| !v.is_empty())
                .map(|v| v.len() as u64))
        })
    }
    fn delete<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, DeleteOutcome> {
        Box::pin(async move {
            if matches!(self.before().await?, Fault::UnknownDelete) {
                return Ok(DeleteOutcome::Unknown);
            }
            self.objects.lock().await.insert(key.clone(), Vec::new());
            Ok(DeleteOutcome::Confirmed)
        })
    }
    fn available(&self) -> IoFuture<'_, Option<u64>> {
        Box::pin(async {
            self.before().await?;
            Ok(None)
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn key(bytes: &[u8], generation: &str) -> ObjectKey {
        ObjectKey {
            root: "root".parse().unwrap(),
            digest: digest(bytes).parse().unwrap(),
            generation: generation.parse().unwrap(),
        }
    }
    pub async fn conformance(store: &dyn ObjectStore) {
        let bytes = b"object payload".to_vec();
        let k = key(&bytes, "generation1");
        store
            .put(&k, bytes.len() as u64, reader(bytes.clone()))
            .await
            .unwrap();
        store
            .put(&k, bytes.len() as u64, reader(bytes.clone()))
            .await
            .unwrap();
        assert_eq!(store.stat(&k).await.unwrap(), Some(bytes.len() as u64));
        assert_eq!(
            verified(store.read(&k).await.unwrap(), &k, bytes.len() as u64)
                .await
                .unwrap(),
            bytes
        );
        assert!(store.put(&k, 3, reader(b"bad".to_vec())).await.is_err());
        let next = key(&bytes, "generation2");
        store
            .put(&next, bytes.len() as u64, reader(bytes.clone()))
            .await
            .unwrap();
        assert_eq!(store.delete(&k).await.unwrap(), DeleteOutcome::Confirmed);
        assert!(
            store
                .put(&k, bytes.len() as u64, reader(bytes.clone()))
                .await
                .is_err()
        );
        assert_eq!(store.delete(&k).await.unwrap(), DeleteOutcome::Confirmed);
        assert_eq!(
            verified(store.read(&next).await.unwrap(), &next, bytes.len() as u64)
                .await
                .unwrap(),
            bytes
        );
        store.delete(&next).await.unwrap();
    }
    #[simple_server::test(host_runtime = true)]
    async fn faults_do_not_turn_stat_into_verification() {
        let store = MemoryStore::default();
        conformance(&store).await;
        let key = key(b"payload", "g");
        store.fault(Fault::LostWriteResponse).await;
        assert!(
            store
                .put(&key, 7, reader(b"payload".to_vec()))
                .await
                .is_err()
        );
        assert_eq!(store.stat(&key).await.unwrap(), Some(7));
        for fault in [Fault::CorruptRead, Fault::TruncateRead] {
            store.fault(fault).await;
            assert!(
                verified(store.read(&key).await.unwrap(), &key, 7)
                    .await
                    .is_err()
            );
        }
        store.fault(Fault::None).await;
        assert_eq!(
            verified(store.read(&key).await.unwrap(), &key, 7)
                .await
                .unwrap(),
            b"payload"
        );
    }
}
