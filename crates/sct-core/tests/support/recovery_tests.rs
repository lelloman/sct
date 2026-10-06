use super::*;
use super::{
    tree_build_tests::{admitted, stage_files},
    tree_publication_tests::seal,
};
use sct_core::{
    local_store::LocalStore,
    payload::{Fault, MemoryStore, ObjectKey, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn independent_recovery_restores_files_and_trees_without_production_or_idp() {
    let (w, a, db) = setup().await;
    let root = root(&w, &a, "/recovery", 4000).await;
    let optical = w
        .create_archive_collection(
            &a,
            &CreateArchiveCollectionRequest {
                request_id: "optical".parse().unwrap(),
                label: "Disc archive".parse().unwrap(),
                media_capacity_bytes: 25_000_000_000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    db::query("UPDATE archive_collections SET next_disc=7 WHERE id=$1")
        .bind(optical.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let base = std::env::temp_dir().join(format!("sct-recovery-{}", uuid::Uuid::new_v4()));
    let artifacts = base.join("artifacts");
    let payloads = base.join("payloads");
    let destination = base.join("restored");
    tokio::fs::create_dir_all(&payloads).await.unwrap();
    tokio::fs::create_dir(&destination).await.unwrap();
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]),
        2,
        3600,
    )
    .unwrap()
    .with_checkpoint_directory(artifacts.clone());
    let upload = ready_bytes(&engine, &w, &a, &root.id, "file", b"committed file")
        .await
        .unwrap();
    let file = engine.commit_upload(&a, &root.id, &upload).await.unwrap();
    engine.tick().await.unwrap();
    let parent = w
        .resolve_path(&a, &root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let directory = mkdir(
        &w,
        &a,
        &root.id,
        &parent.id,
        "tree",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let draft = stage_files(
        &engine,
        &w,
        &a,
        &root.id,
        &directory.id,
        &[b"tree file".to_vec()],
        1,
    )
    .await;
    admitted(&engine, &a, &root.id, &draft).await;
    engine
        .upload_chunk(
            &a,
            &root.id,
            &draft,
            &digest(b"tree file").parse().unwrap(),
            b"tree file".to_vec(),
        )
        .await
        .unwrap();
    seal(&engine, &a, &root.id, &draft).await;
    let tree = engine.commit_upload(&a, &root.id, &draft).await.unwrap();
    engine.tick().await.unwrap();
    // An unfinished upload at capture must never become a committed backup.
    let pending = ready_bytes(&engine, &w, &a, &root.id, "unfinished", b"uncommitted")
        .await
        .unwrap();
    // Checkpoint recovery retains permanent disc history but abandons work and
    // host-local image files. These are logical fixtures, not physical burns.
    use super::archive_planning_tests::{capture, finish, publish_fixture};
    let archived = capture(&w, &a, &optical, &root.id, &tree.version_id).await;
    let archived = finish(&w, &a, &archived).await;
    publish_fixture(&w, &a, &db, &archived, true).await;
    let active = capture(&w, &a, &optical, &root.id, &tree.version_id).await;
    let active = finish(&w, &a, &active).await;
    w.confirm_archive_set(
        &a,
        &optical.id,
        &active.id,
        &ArchiveSetActionRequest {
            request_id: "checkpoint-active".parse().unwrap(),
            expected_revision: active.revision.clone(),
        },
    )
    .await
    .unwrap();
    db::query("INSERT INTO archive_artifacts(id,disc_id,state,relative_path,expires_at) SELECT 'active_'||id,id,'building','old-work',clock_timestamp()+interval '1 day' FROM archive_discs WHERE set_id=$1")
        .bind(active.id.as_str()).execute(&db).await.unwrap();
    db::query("INSERT INTO archive_exports(set_id,reserved_bytes,worker_epoch,server_epoch,worker_expires_at,writer_version,writer_options) VALUES($1,100,4,1,clock_timestamp()+interval '1 hour','fixture','{}')")
        .bind(active.id.as_str()).execute(&db).await.unwrap();
    db::query("INSERT INTO archive_builds(disc_id,artifact_id,attempt,stage,relative_path,server_epoch,worker_epoch) SELECT disc_id,id,'old-attempt','staging','old-work',1,4 FROM archive_artifacts WHERE state='building'")
        .execute(&db).await.unwrap();
    let old_cache = base.join("old-archive-cache");
    tokio::fs::create_dir(&old_cache).await.unwrap();
    tokio::fs::write(old_cache.join("sentinel"), b"keep old host files")
        .await
        .unwrap();
    db::query("INSERT INTO archive_storage(directory,quota_bytes,cache_bytes,reserved_bytes) VALUES($1,10000,100,100)")
        .bind(old_cache.to_str().unwrap()).execute(&db).await.unwrap();
    db::query("UPDATE archive_accounts SET cache_bytes=100,reserved_bytes=100")
        .execute(&db)
        .await
        .unwrap();
    db::query("INSERT INTO archive_downloads(id,artifact_id,principal_id,credential_id,expires_at) SELECT 'download_'||ar.id,ar.id,s.principal_id,s.credential_id,clock_timestamp()+interval '1 hour' FROM archive_artifacts ar JOIN archive_discs d ON d.id=ar.disc_id JOIN archive_sets s ON s.id=d.set_id WHERE s.id=$1")
        .bind(archived.id.as_str()).execute(&db).await.unwrap();
    let checkpoint = w
        .create_checkpoint(
            &a,
            &CreateCheckpointRequest {
                idempotency_key: "recover".parse().unwrap(),
            },
        )
        .await
        .unwrap();
    for _ in 0..10 {
        engine.tick().await.unwrap();
        if w.checkpoint(&a, &checkpoint.id).await.unwrap().state == CheckpointState::Exported {
            break;
        }
    }
    let package = artifacts.join(checkpoint.id.as_str());
    let inventory = w
        .checkpoint_inventory(
            &a,
            &checkpoint.id,
            &ListRequest {
                limit: 100.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert!(inventory.next_cursor.is_none());
    let copied = sct_core::recovery::copy_checkpoint_payloads(
        &package,
        &BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]),
        &payloads,
    )
    .await
    .unwrap();
    assert_eq!(copied.package_state, RecoveryPackageState::PayloadVerified);
    let object = &inventory.objects[0];
    let key = ObjectKey {
        root: object.root_id.clone(),
        digest: object.digest.clone(),
        generation: object.generation.parse().unwrap(),
    };
    let path = payloads.join(object.backend_id.as_str()).join(key.path());
    let original = tokio::fs::read(&path).await.unwrap();
    let mut corrupt = original.clone();
    corrupt[0] ^= 1;
    tokio::fs::write(&path, corrupt).await.unwrap();
    assert!(
        sct_core::recovery::verify_checkpoint_package(&package, &payloads)
            .await
            .is_err()
    );
    tokio::fs::write(&path, original).await.unwrap();
    let missing = path.with_extension("missing");
    tokio::fs::rename(&path, &missing).await.unwrap();
    assert!(
        sct_core::recovery::verify_checkpoint_package(&package, &payloads)
            .await
            .is_err()
    );
    tokio::fs::rename(&missing, &path).await.unwrap();
    let descriptor_path = package.join("descriptor.json");
    let original = tokio::fs::read(&descriptor_path).await.unwrap();
    let mut unsupported: serde_json::Value = serde_json::from_slice(&original).unwrap();
    unsupported["format"] = json!(99);
    let mut bytes = serde_json::to_vec(&unsupported).unwrap();
    bytes.push(b'\n');
    tokio::fs::write(&descriptor_path, bytes).await.unwrap();
    assert!(
        sct_core::recovery::verify_checkpoint_package(&package, &payloads)
            .await
            .is_err()
    );
    tokio::fs::write(&descriptor_path, original).await.unwrap();
    assert!(
        w.catalog()
            .recover_checkpoint(&package, &payloads, &destination, true)
            .await
            .is_err()
    );
    memory.fault(Fault::Offline).await;
    let (catalog, target) = support::database().await;
    assert!(
        catalog
            .recover_checkpoint(&package, &payloads, &destination, false)
            .await
            .is_err()
    );
    let verified = catalog
        .recover_checkpoint(&package, &payloads, &destination, true)
        .await
        .unwrap();
    assert_eq!(
        verified.package_state,
        RecoveryPackageState::PayloadVerified
    );
    assert!(
        catalog
            .recover_checkpoint(&package, &payloads, &destination, true)
            .await
            .is_err()
    );
    let writer = catalog.acquire_writer().await.unwrap();
    let credential = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&credential.secret, false, None)
        .await
        .unwrap();
    let high_water: i64 = db::query_scalar(
        "SELECT next_disc FROM archive_collections WHERE id=$1 AND reconciliation_required",
    )
    .bind(optical.id.as_str())
    .fetch_one(&target)
    .await
    .unwrap();
    assert_eq!(high_water, 9);
    let historical: bool = db::query_scalar(
        "SELECT state='ready' AND verified_complete_at IS NOT NULL FROM archive_sets WHERE id=$1",
    )
    .bind(archived.id.as_str())
    .fetch_one(&target)
    .await
    .unwrap();
    assert!(historical);
    let abandoned: bool = db::query_scalar("SELECT state='failed' AND failure='archive_reconciliation_required' AND pinned_bytes=0 FROM archive_sets WHERE id=$1")
        .bind(active.id.as_str()).fetch_one(&target).await.unwrap();
    assert!(abandoned);
    for table in ["archive_downloads", "archive_storage", "graph_pins"] {
        let count: i64 = db::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&target)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let unsafe_runtime: bool = db::query_scalar("SELECT EXISTS(SELECT 1 FROM archive_accounts WHERE cache_bytes<>0 OR reserved_bytes<>0 OR pinned_bytes<>0) OR EXISTS(SELECT 1 FROM archive_exports WHERE NOT finished OR reserved_bytes<>0 OR server_epoch<>0 OR worker_expires_at IS NOT NULL) OR EXISTS(SELECT 1 FROM archive_builds WHERE relative_path<>'' OR server_epoch<>0) OR EXISTS(SELECT 1 FROM archive_artifacts WHERE relative_path IS NOT NULL OR state IN ('building','available'))")
        .fetch_one(&target).await.unwrap();
    assert!(!unsafe_runtime);
    let preserved: i64 = db::query_scalar("SELECT count(*) FROM archive_artifacts WHERE state='expired' AND sha256=repeat('a',64) AND length IS NOT NULL")
        .fetch_one(&target).await.unwrap();
    assert_eq!(preserved, 1);
    let copies: i64 =
        db::query_scalar("SELECT count(*) FROM archive_copies WHERE health='read_back_verified'")
            .fetch_one(&target)
            .await
            .unwrap();
    assert_eq!(copies, 1);
    assert_eq!(
        tokio::fs::read(old_cache.join("sentinel")).await.unwrap(),
        b"keep old host files"
    );
    let error = writer
        .create_archive_plan(
            &actor,
            &optical.id,
            &CreateArchivePlanRequest {
                request_id: "after-restore".parse().unwrap(),
                expected_revision: optical.revision.clone(),
                selections: vec![ArchiveSelection {
                    root_id: root.id.clone(),
                    version_id: file.version_id.clone(),
                    path: "/file".parse().unwrap(),
                }],
                media_capacity_bytes: optical.media_capacity_bytes,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.0, ErrorCode::ArchiveReconciliationRequired);
    let mut stores = BTreeMap::new();
    for backend in writer.configured_backends().await.unwrap() {
        assert_eq!(backend.inspection.adapter_kind, "local");
        let path = std::path::Path::new(backend.settings["directory"].as_str().unwrap());
        assert!(path.starts_with(&destination));
        stores.insert(
            backend.inspection.id.as_str().into(),
            Arc::new(LocalStore::new(path, 0).unwrap()) as Arc<dyn ObjectStore>,
        );
    }
    let recovered = TransferEngine::new(writer.clone(), stores, 2, 3600).unwrap();
    for _ in 0..8 {
        recovered.tick().await.unwrap();
    }
    let mut read = recovered
        .download(&actor, &root.id, &file.version_id, None)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = read.next().await.unwrap() {
        bytes.extend(chunk);
    }
    assert_eq!(bytes, b"committed file");
    drop(read);
    let mut read = recovered
        .download_snapshot_file(
            &actor,
            &root.id,
            &tree.version_id,
            &"/file-0".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = read.next().await.unwrap() {
        bytes.extend(chunk);
    }
    assert_eq!(bytes, b"tree file");
    drop(read);
    let pending_state: String = db::query_scalar("SELECT state FROM transfer_sessions WHERE id=$1")
        .bind(pending.as_str())
        .fetch_one(&target)
        .await
        .unwrap();
    assert_eq!(pending_state, "expired");
    assert_eq!(
        writer
            .resolve_path(&actor, &root.id, &"/unfinished".parse().unwrap())
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let new = ready_bytes(
        &recovered,
        &writer,
        &actor,
        &root.id,
        "after-recovery",
        b"new generation",
    )
    .await
    .unwrap();
    recovered
        .commit_upload(&actor, &root.id, &new)
        .await
        .unwrap();
    recovered.tick().await.unwrap();
    let lease_count: i64 = db::query_scalar("SELECT count(*) FROM server_lease")
        .fetch_one(&target)
        .await
        .unwrap();
    assert_eq!(lease_count, 1);
    tokio::fs::remove_dir_all(base).await.unwrap();
}
