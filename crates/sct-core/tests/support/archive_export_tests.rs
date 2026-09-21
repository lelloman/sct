use super::*;
use sct_core::{
    archive_export::ArchiveExportConfig,
    archive_iso::IsoWriter,
    payload::{Fault, MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::io::AsyncReadExt;
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("sct-iso-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let mut stack = vec![self.0.clone()];
        while let Some(path) = stack.pop() {
            if let Ok(meta) = std::fs::symlink_metadata(&path)
                && meta.is_dir()
                && !meta.file_type().is_symlink()
            {
                let _ = std::fs::set_permissions(
                    &path,
                    std::fs::Permissions::from_mode(meta.permissions().mode() | 0o700),
                );
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if entry.file_type().is_ok_and(|t| t.is_dir()) {
                            stack.push(entry.path());
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn hash(path: &Path) -> String {
    let mut f = tokio::fs::File::open(path).await.unwrap();
    let mut b = vec![0; 65536];
    let mut h = Sha256::new();
    loop {
        let n = f.read(&mut b).await.unwrap();
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    format!("{:x}", h.finalize())
}
async fn bundle(root: &Path) -> (PathBuf, u64) {
    let directory = root.join("bundle");
    std::fs::create_dir_all(directory.join("recovery")).unwrap();
    let binary = std::env::var("SCT_TEST_RECOVERY_BINARY")
        .expect("scripts/test-integration builds the recovery binary");
    std::fs::copy(binary, directory.join("recovery/sct-recover")).unwrap();
    std::fs::write(directory.join("START-HERE.txt"),"Integration fixture: use recovery/sct-recover. Production bundles are built by scripts/build-recovery-bundle.\n").unwrap();
    // This test exercises a real executable and ISO; complete vendored source
    // packaging is independently qualified by scripts/test-recovery-bundle.
    std::fs::write(
        directory.join("recovery/source.tar.gz"),
        b"test-only source placeholder",
    )
    .unwrap();
    let mut files = Vec::new();
    let mut size = 0;
    for path in [
        "START-HERE.txt",
        "recovery/sct-recover",
        "recovery/source.tar.gz",
    ] {
        let length = std::fs::metadata(directory.join(path)).unwrap().len();
        size += length;
        files.push(ArchiveBundleFile {
            path: path.into(),
            length: length.try_into().unwrap(),
            sha256: hash(&directory.join(path)).await.parse().unwrap(),
        });
    }
    let manifest = ArchiveBundle {
        format: 1,
        version: "integration".into(),
        platform: "linux-x86_64-glibc236".into(),
        files,
    };
    let bytes = canonical(&manifest).unwrap();
    size += bytes.len() as u64;
    std::fs::write(directory.join("recovery/bundle.json"), bytes).unwrap();
    (directory, size)
}
async fn engine(
    w: &Writer,
    store: &Arc<MemoryStore>,
    config: &ArchiveExportConfig,
) -> TransferEngine {
    TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), store.clone() as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap()
    .with_archive_exports(config.clone())
    .await
    .unwrap()
}
async fn prepared(
    w: &Writer,
    a: &Actor,
    db: &PgPool,
    e: &TransferEngine,
    capacity: u64,
) -> (ArchiveCollection, ArchiveSet, Vec<Vec<u8>>) {
    use super::{
        tree_build_tests::{admitted, stage_files},
        tree_publication_tests::seal,
    };
    sqlx::query("UPDATE pools SET capacity=1073741824")
        .execute(db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=536870912 WHERE root_id IS NULL")
        .execute(db)
        .await
        .unwrap();
    let r = root(w, a, "/iso", 134217728).await;
    let parent = w
        .resolve_path(a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let d = mkdir(
        w,
        a,
        &r.id,
        &parent.id,
        "tree",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let chunks = vec![vec![11; 4_000_000], vec![12; 4_000_000]];
    let draft = stage_files(e, w, a, &r.id, &d.id, &chunks, 1).await;
    admitted(e, a, &r.id, &draft).await;
    for bytes in &chunks {
        e.upload_chunk(
            a,
            &r.id,
            &draft,
            &digest(bytes).parse().unwrap(),
            bytes.clone(),
        )
        .await
        .unwrap();
    }
    seal(e, a, &r.id, &draft).await;
    let receipt = e.commit_upload(a, &r.id, &draft).await.unwrap();
    e.tick().await.unwrap();
    let c = w
        .create_archive_collection(
            a,
            &CreateArchiveCollectionRequest {
                request_id: "collection".parse().unwrap(),
                label: "ISO".parse().unwrap(),
                media_capacity_bytes: capacity.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    let s = w
        .create_archive_plan(
            a,
            &c.id,
            &CreateArchivePlanRequest {
                request_id: "plan".parse().unwrap(),
                expected_revision: c.revision.clone(),
                selections: vec![ArchiveSelection {
                    root_id: r.id,
                    version_id: receipt.version_id,
                    path: "/".parse().unwrap(),
                }],
                media_capacity_bytes: c.media_capacity_bytes,
            },
        )
        .await
        .unwrap();
    for i in 0..100 {
        if i % 16 == 0 {
            w.renew().await.unwrap();
        }
        if !w.advance_archive_capture(&s.id).await.unwrap() {
            break;
        }
    }
    e.archive_tick().await.unwrap();
    let s = w.inspect_archive_set(a, &c.id, &s.id).await.unwrap();
    assert_eq!(s.state, ArchiveJobState::AwaitingConfirmation);
    assert_eq!(s.plan.as_ref().unwrap().new_disc_count.get(), 2);
    let s = w
        .confirm_archive_set(
            a,
            &c.id,
            &s.id,
            &ArchiveSetActionRequest {
                request_id: "confirm".parse().unwrap(),
                expected_revision: s.revision,
            },
        )
        .await
        .unwrap();
    (c, s, chunks)
}
#[tokio::test]
#[ignore = "requires PostgreSQL, xorriso and recovery binary"]
async fn archive_iso_restart_builds_real_images_and_every_disc_bootstraps_recovery() {
    let scratch = Scratch::new();
    let (bundle, size) = bundle(&scratch.0).await;
    let config = ArchiveExportConfig {
        directory: scratch.0.join("exports"),
        bundle_directory: bundle,
        quota_bytes: 1_000_000_000.try_into().unwrap(),
    };
    let (w, a, db) = setup().await;
    let store = Arc::new(MemoryStore::default());
    let e = engine(&w, &store, &config).await;
    let (c, s, chunks) = prepared(&w, &a, &db, &e, size + 6_200_000).await;
    store
        .fault(Fault::Delay(std::time::Duration::from_secs(60)))
        .await;
    let running = tokio::spawn({
        let e = e.clone();
        async move { e.archive_tick().await }
    });
    let mut staged = false;
    for _ in 0..300 {
        if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_builds")
            .fetch_one(&db)
            .await
            .unwrap()
            > 0
        {
            staged = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(staged);
    running.abort();
    let _ = running.await;
    store.fault(Fault::None).await;
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let _heartbeat = LeaseHeartbeat(tokio::spawn({
        let w = w.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if w.renew().await.is_err() {
                    break;
                }
            }
        }
    }));
    let e = engine(&w, &store, &config).await;
    for _ in 0..5 {
        w.renew().await.unwrap();
        e.archive_tick().await.unwrap();
        if w.inspect_archive_set(&a, &c.id, &s.id).await.unwrap().state == ArchiveJobState::Ready {
            break;
        }
    }
    assert_eq!(
        w.inspect_archive_set(&a, &c.id, &s.id).await.unwrap().state,
        ArchiveJobState::Ready
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT reserved_bytes FROM archive_accounts")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(config.directory.join("work"))
            .unwrap()
            .count(),
        0
    );
    let images=sqlx::query("SELECT a.*,d.planned_iso_bytes FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id WHERE d.set_id=$1 ORDER BY d.number").bind(s.id.as_str()).fetch_all(&db).await.unwrap();
    assert_eq!(images.len(), 2);
    let cache = scratch.0.join("restore-cache");
    std::fs::create_dir(&cache).unwrap();
    let destination = scratch.0.join("restored");
    std::fs::create_dir(&destination).unwrap();
    let mut readers = Vec::new();
    let mut media = Vec::new();
    let mut total = 0;
    for (i, row) in images.iter().enumerate() {
        let path = config.directory.join(row.get::<String, _>("relative_path"));
        let length = std::fs::metadata(&path).unwrap().len();
        total += length;
        assert_eq!(length, row.get::<i64, _>("length") as u64);
        assert!(length <= row.get::<i64, _>("planned_iso_bytes") as u64);
        assert!(length <= c.media_capacity_bytes.get());
        assert_eq!(hash(&path).await, row.get::<String, _>("sha256"));
        let extracted = scratch.0.join(format!("disc-{i}"));
        IsoWriter::default()
            .extract(&path, &extracted, c.media_capacity_bytes.get())
            .await
            .unwrap();
        let mut reader = sct_archive::ArchiveIndex::create(
            &extracted,
            &scratch.0.join(format!("index-{i}.sqlite")),
        )
        .await
        .unwrap();
        assert!(
            reader
                .verify_media(&extracted, &mut Vec::new())
                .await
                .unwrap()
                .complete()
        );
        let output = std::process::Command::new(extracted.join("recovery/sct-recover"))
            .arg("inspect")
            .arg(&extracted)
            .arg(scratch.0.join(format!("cli-{i}.sqlite")))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        media.push(extracted);
        readers.push(reader);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT cache_bytes FROM archive_accounts")
            .fetch_one(&db)
            .await
            .unwrap() as u64,
        total
    );
    let mut reader = readers.pop().unwrap();
    for disc in &media {
        reader.ingest(disc, &cache, &mut Vec::new()).await.unwrap();
    }
    let result = reader
        .restore_to(&cache, &destination, false, &mut Vec::new())
        .await
        .unwrap();
    assert!(result.complete);
    for (i, bytes) in chunks.iter().enumerate() {
        assert_eq!(
            hash(&destination.join(format!("file-{i}"))).await,
            digest(bytes)
        );
    }
    assert!(destination.join("empty-0000").is_dir());
    let hashes: Vec<_> = images
        .iter()
        .map(|r| r.get::<String, _>("sha256"))
        .collect();
    e.archive_tick().await.unwrap();
    let again:Vec<String>=sqlx::query_scalar("SELECT a.sha256 FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id WHERE d.set_id=$1 ORDER BY d.number").bind(s.id.as_str()).fetch_all(&db).await.unwrap();
    assert_eq!(hashes, again);
}
#[tokio::test]
#[ignore = "requires PostgreSQL, xorriso and recovery binary"]
async fn archive_iso_corrupt_source_and_quota_failure_release_pins_and_reservations() {
    for quota_failure in [false, true] {
        let scratch = Scratch::new();
        let (bundle, size) = bundle(&scratch.0).await;
        let config = ArchiveExportConfig {
            directory: scratch.0.join("exports"),
            bundle_directory: bundle,
            quota_bytes: 1_000_000_000.try_into().unwrap(),
        };
        let (w, a, db) = setup().await;
        let store = Arc::new(MemoryStore::default());
        let e = engine(&w, &store, &config).await;
        let (c, s, _) = prepared(&w, &a, &db, &e, size + 6_200_000).await;
        if quota_failure {
            sqlx::query("UPDATE archive_accounts SET quota_bytes=1")
                .execute(&db)
                .await
                .unwrap();
        } else {
            store.fault(Fault::CorruptRead).await;
        }
        e.archive_tick().await.unwrap();
        let state = w.inspect_archive_set(&a, &c.id, &s.id).await.unwrap();
        assert_eq!(state.state, ArchiveJobState::Failed);
        assert_eq!(
            state.failure,
            Some(if quota_failure {
                ErrorCode::ArchiveResourceLimit
            } else {
                ErrorCode::IntegrityFailure
            })
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT reserved_bytes+cache_bytes+pinned_bytes FROM archive_accounts"
            )
            .fetch_one(&db)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT reserved_bytes+cache_bytes FROM archive_storage")
                .fetch_one(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
                .fetch_one(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            std::fs::read_dir(config.directory.join("work"))
                .unwrap()
                .count(),
            0
        );
    }
}
#[tokio::test]
#[ignore = "requires xorriso"]
async fn archive_iso_writer_enforces_process_size_limit_and_never_overwrites() {
    let scratch = Scratch::new();
    let tree = scratch.0.join("tree");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("payload"), vec![7; 8192]).unwrap();
    let writer = IsoWriter::default();
    assert!(writer.version().await.unwrap().contains("xorriso"));
    let small = scratch.0.join("small.iso");
    assert!(writer.write(&tree, &small, 2048).await.is_err());
    assert!(std::fs::metadata(small).unwrap().len() <= 2048);
    let good = scratch.0.join("good.iso");
    writer.write(&tree, &good, 2_000_000).await.unwrap();
    let before = hash(&good).await;
    assert_eq!(
        writer.write(&tree, &good, 2_000_000).await.unwrap_err().0,
        ErrorCode::RevisionConflict
    );
    assert_eq!(hash(&good).await, before);
}

struct LeaseHeartbeat(tokio::task::JoinHandle<()>);
impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL, xorriso and recovery binary"]
async fn archive_iso_cancellation_fences_live_staging_before_releasing_reservations() {
    let scratch = Scratch::new();
    let (bundle, size) = bundle(&scratch.0).await;
    let config = ArchiveExportConfig {
        directory: scratch.0.join("exports"),
        bundle_directory: bundle,
        quota_bytes: 1_000_000_000.try_into().unwrap(),
    };
    let (w, a, db) = setup().await;
    let store = Arc::new(MemoryStore::default());
    let e = engine(&w, &store, &config).await;
    let (c, s, _) = prepared(&w, &a, &db, &e, size + 6_200_000).await;
    store
        .fault(Fault::Delay(std::time::Duration::from_secs(60)))
        .await;
    let running = tokio::spawn({
        let e = e.clone();
        async move { e.archive_tick().await }
    });
    let mut started = false;
    for _ in 0..300 {
        if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_builds")
            .fetch_one(&db)
            .await
            .unwrap()
            > 0
        {
            started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(started);
    let current = w.inspect_archive_set(&a, &c.id, &s.id).await.unwrap();
    w.cancel_archive_set(
        &a,
        &c.id,
        &s.id,
        &ArchiveSetActionRequest {
            request_id: "cancel-live-export".parse().unwrap(),
            expected_revision: current.revision,
        },
    )
    .await
    .unwrap();
    running.await.unwrap().unwrap();
    assert_eq!(
        w.inspect_archive_set(&a, &c.id, &s.id).await.unwrap().state,
        ArchiveJobState::Cancelled
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT reserved_bytes+cache_bytes+pinned_bytes FROM archive_accounts"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM archive_artifacts WHERE state='available'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(config.directory.join("work"))
            .unwrap()
            .count(),
        0
    );
}
