use super::*;
use super::{
    tree_build_tests::{admitted, stage_files},
    tree_publication_tests::seal,
};
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn hide_directory_preserves_charges_histories_and_latest_protection() {
    let (w, a, db) = setup().await;
    let mut req = request(a.principal_id(), "/hide", 4000);
    req.quota_bytes = 12.try_into().unwrap();
    req.default_policy.mode = RetentionMode::Relaxed;
    let r = w.create_root(&a, &req).await.unwrap();
    let r = w
        .update_root(
            &a,
            &r.id,
            &UpdateRootRequest {
                expected_revision: r.revision.clone(),
                display_name: r.display_name.clone(),
                state: RootState::Enabled,
                placement: r.placement.clone(),
                quota_bytes: r.quota_bytes,
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let e = TransferEngine::new(
        w.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        60,
    )
    .unwrap();
    let upload = ready_bytes(&e, &w, &a, &r.id, "plain", b"data")
        .await
        .unwrap();
    let file_version = e.commit_upload(&a, &r.id, &upload).await.unwrap();
    let file = w
        .resolve_path(&a, &r.id, &"/plain".parse().unwrap())
        .await
        .unwrap();
    let parent = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let folder = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "folder",
        VersioningMode::IndependentFiles,
    )
    .await;
    let moved = w
        .rename_entry(
            &a,
            &file.id,
            &rename_request(&w, &a, &file, &folder.id, "plain").await,
        )
        .await
        .unwrap();
    let atomic = mkdir(
        &w,
        &a,
        &r.id,
        &folder.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let draft = stage_files(&e, &w, &a, &r.id, &atomic.id, &[b"tree".to_vec()], 0).await;
    admitted(&e, &a, &r.id, &draft).await;
    e.upload_chunk(
        &a,
        &r.id,
        &draft,
        &digest(b"tree").parse().unwrap(),
        b"tree".to_vec(),
    )
    .await
    .unwrap();
    seal(&e, &a, &r.id, &draft).await;
    let snapshot = e.commit_upload(&a, &r.id, &draft).await.unwrap();
    let child = w
        .snapshot_entry(
            &a,
            &r.id,
            &snapshot.version_id,
            &["file-0".parse().unwrap()],
        )
        .await
        .unwrap();
    assert_eq!(
        w.hide_entry(
            &a,
            &r.id,
            &child.id,
            &HideEntryRequest {
                idempotency_key: "no-child-hide".parse().unwrap(),
                expected_revision: "none".parse().unwrap()
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    let before: i64 = sqlx::query_scalar("SELECT sum(occupied)::bigint FROM allocations")
        .fetch_one(&db)
        .await
        .unwrap();
    let request = HideEntryRequest {
        idempotency_key: "hide-folder".parse().unwrap(),
        expected_revision: w.inspect_entry(&a, &folder.id).await.unwrap().revision,
    };
    let hidden = w.hide_entry(&a, &r.id, &folder.id, &request).await.unwrap();
    assert!(hidden.hidden);
    assert_eq!(
        w.hide_entry(&a, &r.id, &folder.id, &request).await.unwrap(),
        hidden
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM entries WHERE hidden")
            .fetch_one(&db)
            .await
            .unwrap(),
        1,
        "only the ancestor changes visibility"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT sum(occupied)::bigint FROM allocations")
            .fetch_one(&db)
            .await
            .unwrap(),
        before
    );
    assert!(w.inspect_entry(&a, &file.id).await.unwrap().hidden);
    assert!(w.inspect_entry(&a, &atomic.id).await.unwrap().hidden);
    assert_eq!(
        w.resolve_path(&a, &r.id, &"/folder/plain".parse().unwrap())
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert!(w.current_snapshot(&a, &r.id, &atomic.id).await.is_err());
    let list = ListEntriesRequest {
        limit: 10.try_into().unwrap(),
        cursor: None,
        include_hidden: false,
    };
    assert!(
        w.list_entries(&a, &r.id, &parent.id, &list)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        w.list_entries(&a, &r.id, &folder.id, &list)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let deleted = w
        .list_entries(
            &a,
            &r.id,
            &folder.id,
            &ListEntriesRequest {
                include_hidden: true,
                ..list
            },
        )
        .await
        .unwrap();
    assert_eq!(deleted.items.len(), 2);
    assert!(deleted.items.iter().all(|e| e.hidden));
    let mut read = e
        .download(&a, &r.id, &file_version.version_id, None)
        .await
        .unwrap();
    assert_eq!(read.next().await.unwrap().unwrap(), b"data");
    drop(read);
    let mut read = e
        .download_snapshot_file(
            &a,
            &r.id,
            &snapshot.version_id,
            &"/file-0".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(read.next().await.unwrap().unwrap(), b"tree");
    drop(read);
    let recreated = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "folder",
        VersioningMode::IndependentFiles,
    )
    .await;
    assert_ne!(recreated.id, folder.id);
    let policy = w
        .effective_policy(
            &a,
            &PolicyScope::Entry {
                root_id: r.id.clone(),
                entry_id: file.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        e.create_upload(
            &a,
            &r.id,
            &CreateUploadRequest {
                idempotency_key: "hidden-replace".parse().unwrap(),
                target: FileTarget::Replace {
                    entry_id: file.id.clone(),
                    expected_revision: moved.revision
                },
                expected_policy_revision: policy.revision,
                manifest: FileManifest {
                    format: 1,
                    profile: CHUNK_PROFILE.into(),
                    length: 0.try_into().unwrap(),
                    chunks: 0.try_into().unwrap(),
                    height: 0,
                    tree: None
                }
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    let incoming = plan_bytes(&e, &w, &a, &r.id, "new", b"12345678")
        .await
        .unwrap();
    let mut rejected = false;
    for _ in 0..20 {
        if let Err(error) = e.admit_upload(&a, &r.id, &incoming).await {
            assert_eq!(error.0, ErrorCode::QuotaExhausted);
            rejected = true;
            break;
        }
    }
    assert!(rejected);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM file_versions WHERE NOT retired")
            .fetch_one(&db)
            .await
            .unwrap(),
        2,
        "hidden latest versions cannot be relaxed-pruned"
    );
}
