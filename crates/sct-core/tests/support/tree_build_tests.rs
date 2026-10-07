use super::tree_draft_tests::{draft_request, mutation};
use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

pub(super) async fn stage_files(
    e: &TransferEngine,
    w: &Writer,
    a: &Actor,
    r: &RootId,
    d: &EntryId,
    files: &[Vec<u8>],
    empty_directories: usize,
) -> UploadId {
    let draft = e
        .create_tree_draft(
            a,
            r,
            &draft_request(w, a, r, d, DraftMode::CompleteReplacement).await,
        )
        .await
        .unwrap();
    let ids = e
        .allocate_draft_identities(
            a,
            r,
            &draft.id,
            &AllocateDraftIdentitiesRequest {
                idempotency_key: "ids".parse().unwrap(),
                count: ((files.len() + empty_directories) as u64)
                    .try_into()
                    .unwrap(),
            },
        )
        .await
        .unwrap();
    let mut changes = Vec::new();
    for (i, bytes) in files.iter().enumerate() {
        let page = canonical(&ManifestPage::Chunks {
            format: 1,
            chunks: vec![ChunkReference {
                digest: digest(bytes).parse().unwrap(),
                length: (bytes.len() as u64).try_into().unwrap(),
            }],
        })
        .unwrap();
        let page_id = digest(&page);
        e.tree_draft_page(a, r, &draft.id, &page_id.parse().unwrap(), &page)
            .await
            .unwrap();
        changes.push(DraftMutation::Upsert {
            identity: DraftEntryIdentity::New {
                entry_id: ids.entry_ids[i].clone(),
            },
            parent_id: d.clone(),
            name: format!("file-{i}").parse().unwrap(),
            entry_kind: EntryKind::File,
            file: Some(FileManifest {
                format: 1,
                profile: CHUNK_PROFILE.into(),
                length: (bytes.len() as u64).try_into().unwrap(),
                chunks: 1.try_into().unwrap(),
                height: 0,
                tree: Some(page_id.parse().unwrap()),
            }),
        });
    }
    for i in 0..empty_directories {
        changes.push(DraftMutation::Upsert {
            identity: DraftEntryIdentity::New {
                entry_id: ids.entry_ids[files.len() + i].clone(),
            },
            parent_id: d.clone(),
            name: format!("empty-{i:04}").parse().unwrap(),
            entry_kind: EntryKind::Directory,
            file: None,
        });
    }
    e.mutate_tree_draft(a, r, &draft.id, &mutation(changes))
        .await
        .unwrap();
    e.freeze_tree_draft(a, r, &draft.id).await.unwrap();
    draft.id
}

pub(super) async fn admitted(
    e: &TransferEngine,
    a: &Actor,
    r: &RootId,
    draft: &UploadId,
) -> TreeDraftStatus {
    for _ in 0..1200 {
        let status = match e.admit_tree_draft(a, r, draft).await {
            Ok(status) => status,
            Err(error)
                if matches!(
                    error.0,
                    ErrorCode::QuotaExhausted
                        | ErrorCode::StagingExhausted
                        | ErrorCode::DestinationCapacity
                ) =>
            {
                let status = e.tree_draft_status(a, r, draft).await.unwrap();
                assert_eq!(status.admission, AdmissionState::Rejected);
                status
            }
            Err(error) => panic!("unexpected admission error: {error}"),
        };
        if matches!(
            status.admission,
            AdmissionState::Admitted | AdmissionState::Rejected
        ) {
            return status;
        }
    }
    panic!("tree admission did not converge");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn whole_tree_admission_counts_unique_chunks_before_accepting_payload() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/tree-admit", 4000).await;
    sqlx::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let parent = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let d = mkdir(
        &w,
        &a,
        &r.id,
        &parent.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let stores = BTreeMap::from([(
        "one".into(),
        Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
    )]);
    let e = TransferEngine::new(w.clone(), stores.clone(), 2, 60).unwrap();
    let files = vec![vec![1; 2100], vec![2; 2100]];
    let draft = stage_files(&e, &w, &a, &r.id, &d.id, &files, 0).await;
    assert!(
        e.chunk_permission(&a, &r.id, &draft, &digest(&files[0]).parse().unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        admitted(&e, &a, &r.id, &draft).await.admission,
        AdmissionState::Rejected
    );
    assert_eq!(
        e.upload_status(&a, &r.id, &draft).await.unwrap().failure,
        Some(ErrorCode::QuotaExhausted)
    );
    assert!(
        e.upload_chunk(
            &a,
            &r.id,
            &draft,
            &digest(&files[0]).parse().unwrap(),
            files[0].clone()
        )
        .await
        .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM object_copies")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    e.tick().await.unwrap();
    // Identical files charge unique bytes once; enough entries force multiple
    // canonical leaf pages and a branch without flattening retained snapshots.
    let files = vec![vec![3; 2100], vec![3; 2100]];
    let draft = stage_files(&e, &w, &a, &r.id, &d.id, &files, 130).await;
    e.admit_tree_draft(&a, &r.id, &draft).await.unwrap();
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), stores, 2, 60).unwrap();
    assert_eq!(
        admitted(&e, &a, &r.id, &draft).await.admission,
        AdmissionState::Admitted
    );
    let manifest:Vec<u8>=sqlx::query_scalar("SELECT o.body FROM tree_drafts d JOIN content_objects o ON o.root_id=d.root_id AND o.digest=d.manifest WHERE d.id=$1").bind(draft.as_str()).fetch_one(&db).await.unwrap();
    let manifest: DirectoryManifest = parse_canonical(&manifest).unwrap();
    assert_eq!(manifest.entries.get(), 132);
    assert_eq!(manifest.height, 1);
    assert_eq!(manifest.length.get(), 4200);
    assert_eq!(
        e.chunk_permission(&a, &r.id, &draft, &digest(&files[0]).parse().unwrap())
            .await
            .unwrap(),
        2100
    );
    e.upload_chunk(
        &a,
        &r.id,
        &draft,
        &digest(&files[0]).parse().unwrap(),
        files[0].clone(),
    )
    .await
    .unwrap();
    for _ in 0..20 {
        if e.seal_upload(&a, &r.id, &draft).await.unwrap().upload.state == SessionState::Ready {
            break;
        }
    }
    assert_eq!(
        e.tree_draft_status(&a, &r.id, &draft).await.unwrap().state,
        SessionState::Ready
    );
    e.abort_tree_draft(&a, &r.id, &draft).await.unwrap();
    for _ in 0..20 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT occupied+reserved FROM allocations WHERE root_id IS NULL"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM draft_entries")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE allocations SET capacity=1000 WHERE root_id IS NULL")
        .execute(&db)
        .await
        .unwrap();
    let files = vec![vec![4; 500], vec![5; 500], vec![6; 500], vec![7; 500]];
    let draft = stage_files(&e, &w, &a, &r.id, &d.id, &files, 0).await;
    assert_eq!(
        admitted(&e, &a, &r.id, &draft).await.admission,
        AdmissionState::Rejected
    );
    assert_eq!(
        e.upload_status(&a, &r.id, &draft).await.unwrap().failure,
        Some(ErrorCode::StagingExhausted)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT capacity FROM allocations WHERE root_id IS NULL")
            .fetch_one(&db)
            .await
            .unwrap(),
        1000
    );
}
