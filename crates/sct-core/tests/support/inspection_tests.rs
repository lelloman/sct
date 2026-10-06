use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use simple_server::database::postgres as db;
use std::collections::{BTreeMap, BTreeSet};
fn page(limit: u64) -> ListRequest {
    ListRequest {
        limit: limit.try_into().unwrap(),
        cursor: None,
    }
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn inspection_distinguishes_usage_and_scopes_program_activity() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/inspection", 4000).await;
    let other = root(&w, &a, "/other-inspection", 4000).await;
    let engine = TransferEngine::new(
        w.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        3600,
    )
    .unwrap();
    for name in ["first", "second"] {
        let upload = ready_bytes(&engine, &w, &a, &r.id, name, b"abc")
            .await
            .unwrap();
        engine.commit_upload(&a, &r.id, &upload).await.unwrap();
        engine.tick().await.unwrap();
    }
    let usage = w.root_usage(&a, &r.id).await.unwrap();
    assert_eq!(usage.logical_retained_bytes.get(), 6);
    assert_eq!(usage.unique_retained_bytes.get(), 3);
    assert!(usage.physical_occupied_bytes.get() > 3); // Metadata is physically stored too.
    assert_eq!(usage.staging_occupied_bytes.get(), 0);
    assert_eq!(usage.reserved_bytes.get(), 0);
    assert_eq!(usage.pending_cleanup_bytes.get(), 0);
    let charged:i64=db::query_scalar("SELECT coalesce(sum(bytes),0)::bigint FROM object_copies WHERE root_id=$1 AND state<>'deleted'").bind(r.id.as_str()).fetch_one(&db).await.unwrap();
    assert_eq!(usage.physical_occupied_bytes.get(), charged as u64);
    let directory = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let tree = mkdir(
        &w,
        &a,
        &r.id,
        &directory.id,
        "tree",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let draft = engine
        .create_tree_draft(
            &a,
            &r.id,
            &super::tree_draft_tests::draft_request(
                &w,
                &a,
                &r.id,
                &tree.id,
                DraftMode::CompleteReplacement,
            )
            .await,
        )
        .await
        .unwrap();
    let all = w
        .list_transfer_activity(&a, &r.id, &page(100))
        .await
        .unwrap();
    assert_eq!(all.sessions.len(), 3);
    assert!(
        all.sessions
            .iter()
            .any(|s| s.id == draft.id && s.kind == "directory" && s.admission.is_none())
    );
    let first = w.list_transfer_activity(&a, &r.id, &page(1)).await.unwrap();
    let next = ListRequest {
        cursor: first.next_cursor.clone(),
        ..page(1)
    };
    let second = w.list_transfer_activity(&a, &r.id, &next).await.unwrap();
    assert_ne!(first.sessions[0].id, second.sessions[0].id);
    assert_eq!(
        w.list_transfer_activity(&a, &other.id, &next)
            .await
            .unwrap_err()
            .0,
        ErrorCode::CursorRefreshRequired
    );
    w.create_principal(&a, &"reader".parse().unwrap())
        .await
        .unwrap();
    let credential = w.issue_token(&a, &"reader".parse().unwrap()).await.unwrap();
    let reader = w
        .authenticate(&credential.secret, false, None)
        .await
        .unwrap();
    w.create_grant(
        &a,
        &CreateGrantRequest {
            root_id: r.id.clone(),
            principal_id: "reader".parse().unwrap(),
            capabilities: vec![Capability::ListRead],
        },
    )
    .await
    .unwrap();
    assert!(w.root_usage(&reader, &r.id).await.is_ok());
    assert!(
        w.list_transfer_activity(&reader, &r.id, &page(1))
            .await
            .is_ok()
    );
    assert!(
        w.list_job_activity(&reader, Some(&r.id), &page(100))
            .await
            .unwrap()
            .jobs
            .len()
            >= 3
    );
    assert_eq!(
        w.root_usage(&reader, &other.id).await.unwrap_err().0,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.list_job_activity(&reader, None, &page(1))
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        w.list_audit(&reader, None, &page(1)).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        engine
            .upload_status(&reader, &r.id, &first.sessions[0].id)
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    // Equal event timestamps must not duplicate or skip events between streams.
    db::query("UPDATE management_audit SET created_at='2026-01-01T00:00:00Z'")
        .execute(&db)
        .await
        .unwrap();
    db::query("UPDATE transfer_audit SET created_at='2026-01-01T00:00:00Z'")
        .execute(&db)
        .await
        .unwrap();
    let expected:i64=db::query_scalar("SELECT (SELECT count(*) FROM management_audit WHERE root_id=$1)+(SELECT count(*) FROM transfer_audit WHERE root_id=$1)").bind(r.id.as_str()).fetch_one(&db).await.unwrap();
    let mut request = page(1);
    let mut ids = BTreeSet::new();
    loop {
        let response = w.list_audit(&reader, Some(&r.id), &request).await.unwrap();
        for event in response.events {
            assert_eq!(event.root_id.as_ref(), Some(&r.id));
            assert!(ids.insert(event.id));
        }
        request.cursor = response.next_cursor;
        if request.cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), expected as usize);
    assert!(ids.iter().any(|id| id.starts_with("transfer-")));
    assert!(ids.iter().any(|id| id.starts_with("management-")));
    w.revoke_credential(&reader, &credential.id).await.unwrap();
    assert_eq!(
        w.list_audit(&reader, Some(&r.id), &page(1))
            .await
            .unwrap_err()
            .0,
        ErrorCode::Unauthorized
    );
}
