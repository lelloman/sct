use super::*;
use super::{
    tree_build_tests::{admitted, stage_files},
    tree_publication_tests::{patch, seal},
};
use sct_core::{
    payload::{Fault, MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::BTreeMap;

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn snapshot_browse_and_pinned_descendant_range_reads_use_immutable_paths() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/tree-read", 4000).await;
    db::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let mut policy = match r.placement.clone() {
        Placement::Single { read_policy, .. } => read_policy,
        _ => unreachable!(),
    };
    let mut second = policy.candidates[0].clone();
    second.backend_id = "two".parse().unwrap();
    policy.candidates.push(second);
    w.update_root(
        &a,
        &r.id,
        &UpdateRootRequest {
            expected_revision: r.revision.clone(),
            display_name: r.display_name.clone(),
            state: RootState::Enabled,
            quota_bytes: r.quota_bytes,
            placement: Placement::Mirror {
                backend_ids: ["one".parse().unwrap(), "two".parse().unwrap()],
                read_policy: policy,
            },
        },
    )
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
    let one = Arc::new(MemoryStore::default());
    let two = Arc::new(MemoryStore::default());
    let e = TransferEngine::new(
        w.clone(),
        BTreeMap::from([
            ("one".into(), one.clone() as Arc<dyn ObjectStore>),
            ("two".into(), two as Arc<dyn ObjectStore>),
        ]),
        3,
        60,
    )
    .unwrap();
    let bytes = b"verified descendant bytes".to_vec();
    let draft = stage_files(&e, &w, &a, &r.id, &d.id, std::slice::from_ref(&bytes), 2).await;
    admitted(&e, &a, &r.id, &draft).await;
    e.upload_chunk(
        &a,
        &r.id,
        &draft,
        &digest(&bytes).parse().unwrap(),
        bytes.clone(),
    )
    .await
    .unwrap();
    seal(&e, &a, &r.id, &draft).await;
    let receipt = e.commit_upload(&a, &r.id, &draft).await.unwrap();
    assert_eq!(
        w.current_snapshot(&a, &r.id, &d.id).await.unwrap(),
        receipt.version_id
    );
    let q = SnapshotListQuery {
        path: "/".parse().unwrap(),
        limit: 1.try_into().unwrap(),
        cursor: None,
    };
    let first = w
        .snapshot_entries(&a, &r.id, &receipt.version_id, &q)
        .await
        .unwrap();
    assert_eq!(first.entries[0].name.as_str(), "empty-0000");
    let next = w
        .snapshot_entries(
            &a,
            &r.id,
            &receipt.version_id,
            &SnapshotListQuery {
                cursor: first.next_cursor.clone(),
                ..q.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(next.entries[0].name.as_str(), "empty-0001");
    let last = w
        .snapshot_entries(
            &a,
            &r.id,
            &receipt.version_id,
            &SnapshotListQuery {
                cursor: next.next_cursor,
                ..q.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(last.entries[0].name.as_str(), "file-0");
    assert!(last.next_cursor.is_none());
    let empty = w
        .snapshot_entries(
            &a,
            &r.id,
            &receipt.version_id,
            &SnapshotListQuery {
                path: "/empty-0000".parse().unwrap(),
                ..q.clone()
            },
        )
        .await
        .unwrap();
    assert!(empty.entries.is_empty());
    assert_eq!(
        w.snapshot_entries(
            &a,
            &r.id,
            &receipt.version_id,
            &SnapshotListQuery {
                path: "/empty-0000".parse().unwrap(),
                cursor: first.next_cursor,
                ..q
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::CursorRefreshRequired
    );
    let resolved = w
        .resolve_path(&a, &r.id, &"/atomic/file-0".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(resolved.id, last.entries[0].id);
    assert!(resolved.current_version_id.is_none());
    let foreign = root(&w, &a, "/foreign", 4000).await;
    assert_eq!(
        w.snapshot_entries(
            &a,
            &foreign.id,
            &receipt.version_id,
            &SnapshotListQuery {
                path: "/".parse().unwrap(),
                limit: 1.try_into().unwrap(),
                cursor: None
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::NotFound
    );
    assert!(
        e.download_snapshot_file(
            &a,
            &foreign.id,
            &receipt.version_id,
            &"/file-0".parse().unwrap(),
            None
        )
        .await
        .is_err()
    );
    let mut reader = e
        .download_snapshot_file(
            &a,
            &r.id,
            &receipt.version_id,
            &"/file-0".parse().unwrap(),
            Some((2, 11)),
        )
        .await
        .unwrap();
    let replacement = patch(&e, &w, &a, &r.id, &d.id, Some(resolved.id)).await;
    seal(&e, &a, &r.id, &replacement).await;
    let newer = e.commit_upload(&a, &r.id, &replacement).await.unwrap();
    assert!(
        e.download_snapshot_file(
            &a,
            &r.id,
            &receipt.version_id,
            &"/file-0".parse().unwrap(),
            None
        )
        .await
        .is_err()
    );
    assert!(
        e.download_snapshot_file(
            &a,
            &r.id,
            &newer.version_id,
            &"/file-0".parse().unwrap(),
            None
        )
        .await
        .is_err()
    );
    // The already pinned reader survives keep-one retirement; corrupt primary
    // metadata/payload falls back to the second verified mirror copy.
    one.fault(Fault::CorruptRead).await;
    let mut actual = Vec::new();
    while let Some(chunk) = reader.next().await.unwrap() {
        actual.extend(chunk);
    }
    assert_eq!(actual, &bytes[2..11]);
    drop(reader);
}
