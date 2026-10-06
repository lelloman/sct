use super::*;
use sct_core::{
    archive_export::ArchiveExportConfig,
    archive_iso::IsoWriter,
    payload::{Fault, MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use sha2::{Digest, Sha256};
use simple_server::database::postgres as db;
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
    db::query("UPDATE pools SET capacity=1073741824")
        .execute(db)
        .await
        .unwrap();
    db::query("UPDATE allocations SET capacity=536870912 WHERE root_id IS NULL")
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
#[simple_server::test(host_runtime = true)]
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
    let running = sct_spawn({
        let e = e.clone();
        async move { e.archive_tick().await }
    });
    let mut staged = false;
    for _ in 0..300 {
        if db::query_scalar::<_, i64>("SELECT count(*) FROM archive_builds")
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
    let _heartbeat = LeaseHeartbeat(sct_spawn({
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
        db::query_scalar::<_, i64>("SELECT reserved_bytes FROM archive_accounts")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
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
    let images=db::query("SELECT a.*,d.planned_iso_bytes,d.inventory_sha256,d.object_count,d.payload_bytes FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id WHERE d.set_id=$1 ORDER BY d.number").bind(s.id.as_str()).fetch_all(&db).await.unwrap();
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
        let observation = reader
            .verify_disc_report(&extracted, &mut std::io::sink())
            .await
            .unwrap();
        assert!(observation.metadata_verified);
        assert_eq!(
            observation.payload_inventory_sha256.as_str(),
            row.get::<String, _>("inventory_sha256")
        );
        assert_eq!(
            observation.expected_objects.get(),
            row.get::<i64, _>("object_count") as u64
        );
        assert_eq!(
            observation.verified_bytes.get(),
            row.get::<i64, _>("payload_bytes") as u64
        );
        let copy = w
            .register_archive_copy(
                &a,
                &c.id,
                &observation.disc_id,
                &RegisterArchiveCopyRequest {
                    request_id: format!("copy-{i}").parse().unwrap(),
                    label: format!("copy {i}").parse().unwrap(),
                    location: "Integration fixture".into(),
                    reported_burned_at_ms: None,
                },
            )
            .await
            .unwrap();
        let report = w
            .submit_archive_verification(
                &a,
                &c.id,
                &copy.id,
                &SubmitArchiveVerificationRequest {
                    request_id: format!("report-{i}").parse().unwrap(),
                    expected_revision: copy.revision,
                    collection_id: c.id.clone(),
                    disc_id: observation.disc_id,
                    index_sha256: observation.index_sha256,
                    bundle_sha256: observation.bundle_sha256,
                    payload_inventory_sha256: observation.payload_inventory_sha256,
                    expected_objects: observation.expected_objects,
                    verified_objects: observation.verified_objects,
                    verified_bytes: observation.verified_bytes,
                    metadata_verified: observation.metadata_verified,
                    verifier_version: observation.verifier_version,
                },
            )
            .await
            .unwrap();
        assert!(report.complete_read_back);
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
        db::query_scalar::<_, i64>("SELECT cache_bytes FROM archive_accounts")
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
    let again:Vec<String>=db::query_scalar("SELECT a.sha256 FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id WHERE d.set_id=$1 ORDER BY d.number").bind(s.id.as_str()).fetch_all(&db).await.unwrap();
    assert_eq!(hashes, again);
    assert!(
        w.inspect_archive_set(&a, &c.id, &s.id)
            .await
            .unwrap()
            .verified_complete_at_ms
            .is_some()
    );
    download_and_expiry_checks(&e, &w, &a, &db, &c, &s, &config, &images).await;
}
#[simple_server::test(host_runtime = true)]
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
            db::query("UPDATE archive_accounts SET quota_bytes=1")
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
            db::query_scalar::<_, i64>(
                "SELECT reserved_bytes+cache_bytes+pinned_bytes FROM archive_accounts"
            )
            .fetch_one(&db)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            db::query_scalar::<_, i64>("SELECT reserved_bytes+cache_bytes FROM archive_storage")
                .fetch_one(&db)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
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
#[simple_server::test(host_runtime = true)]
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

#[simple_server::test(host_runtime = true)]
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
    let running = sct_spawn({
        let e = e.clone();
        async move { e.archive_tick().await }
    });
    let mut started = false;
    for _ in 0..300 {
        if db::query_scalar::<_, i64>("SELECT count(*) FROM archive_builds")
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
        db::query_scalar::<_, i64>(
            "SELECT reserved_bytes+cache_bytes+pinned_bytes FROM archive_accounts"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>(
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

#[allow(clippy::too_many_arguments)]
async fn download_and_expiry_checks(
    e: &TransferEngine,
    w: &Writer,
    actor: &Actor,
    db: &PgPool,
    collection: &ArchiveCollection,
    set: &ArchiveSet,
    config: &ArchiveExportConfig,
    images: &[db::PgRow],
) {
    let disc: ArchiveDiscId = images[0].get::<String, _>("disc_id").parse().unwrap();
    assert_eq!(db::query_scalar::<_,i64>("SELECT count(*) FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id JOIN archive_sets s ON s.id=d.set_id WHERE a.expires_at=s.published_at+interval '7 days'").fetch_one(db).await.unwrap(), 2);
    let artifact = e
        .archive_image_info(actor, &collection.id, &disc)
        .await
        .unwrap()
        .artifact;
    let length = artifact.length.unwrap().get();
    let mut download = e
        .download_archive_iso(actor, &collection.id, &disc, None)
        .await
        .unwrap();
    let mut hash = Sha256::new();
    while let Some(bytes) = download.next().await.unwrap() {
        hash.update(&bytes);
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        artifact.sha256.unwrap().as_str()
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM archive_downloads")
            .fetch_one(db)
            .await
            .unwrap(),
        0
    );
    let path = config
        .directory
        .join(images[0].get::<String, _>("relative_path"));
    let mut file = tokio::fs::File::open(&path).await.unwrap();
    use tokio::io::AsyncSeekExt;
    file.seek(std::io::SeekFrom::Start(length - 128))
        .await
        .unwrap();
    let mut expected = vec![0; 128];
    file.read_exact(&mut expected).await.unwrap();
    let mut download = e
        .download_archive_iso(actor, &collection.id, &disc, Some((length - 128, length)))
        .await
        .unwrap();
    assert_eq!(download.next().await.unwrap().unwrap(), expected);
    download.release().await.unwrap();

    let credential = w
        .issue_token(actor, &actor.principal_id().parse().unwrap())
        .await
        .unwrap();
    let second = w
        .authenticate(&credential.secret, false, None)
        .await
        .unwrap();
    let mut revoked = e
        .download_archive_iso(&second, &collection.id, &disc, None)
        .await
        .unwrap();
    assert!(revoked.next().await.unwrap().is_some());
    w.revoke_credential(actor, &credential.id).await.unwrap();
    assert!(
        revoked.next().await.is_err(),
        "revocation stops the next stream chunk"
    );
    revoked.release().await.unwrap();
    let limits = w
        .archive_account_limits(actor, &actor.principal_id().parse().unwrap())
        .await
        .unwrap();
    let too_low = UpdateArchiveAccountLimitsRequest {
        expected_revision: limits.revision,
        quota_bytes: 0.try_into().unwrap(),
        pin_quota_bytes: limits.pin_quota_bytes,
        max_active: limits.max_active,
        max_capture_records: limits.max_capture_records,
        pin_deadline_seconds: limits.pin_deadline_seconds,
        cache_ttl_seconds: limits.cache_ttl_seconds,
    };
    assert_eq!(
        w.update_archive_account_limits(actor, &limits.principal_id, &too_low)
            .await
            .unwrap_err()
            .0,
        ErrorCode::ArchiveResourceLimit
    );
    let mut live = e
        .download_archive_iso(actor, &collection.id, &disc, None)
        .await
        .unwrap();
    db::query("UPDATE archive_artifacts SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(db)
        .await
        .unwrap();
    e.cleanup_archive_cache().await.unwrap();
    assert!(path.exists(), "live lease holds the image past cache TTL");
    assert!(live.next().await.unwrap().is_some());
    assert!(matches!(
        e.archive_image_info(actor, &collection.id, &disc).await,
        Err(sct_core::error::Error(
            ErrorCode::ArchiveArtifactUnavailable
        ))
    ));
    live.release().await.unwrap();
    // Simulate crash after durable cleanup intent and unlink, before accounting.
    db::query("UPDATE archive_artifacts SET cleanup_pending=true WHERE disc_id=$1")
        .bind(disc.as_str())
        .execute(db)
        .await
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    e.cleanup_archive_cache().await.unwrap();
    e.cleanup_archive_cache().await.unwrap();
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT cache_bytes+reserved_bytes FROM archive_accounts")
            .fetch_one(db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT cache_bytes+reserved_bytes FROM archive_storage")
            .fetch_one(db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::query_scalar::<_, i64>("SELECT count(*) FROM archive_discs")
            .fetch_one(db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        w.inspect_archive_set(actor, &collection.id, &set.id)
            .await
            .unwrap()
            .state,
        ArchiveJobState::Ready
    );
    let hashes: Vec<String> = db::query_scalar("SELECT a.sha256 FROM archive_artifacts a JOIN archive_discs d ON d.id=a.disc_id ORDER BY d.number").fetch_all(db).await.unwrap();
    assert_eq!(
        hashes,
        images
            .iter()
            .map(|r| r.get::<String, _>("sha256"))
            .collect::<Vec<_>>()
    );
}

fn sct_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(
        simple_server::runtime::Runtime::try_current()
            .expect("test engine")
            .scope(future),
    )
}
