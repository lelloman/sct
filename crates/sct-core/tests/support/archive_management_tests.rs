use super::*;
use sct_core::{
    archive_catalog::reserve_disc_numbers,
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;
async fn user(w: &Writer, admin: &Actor, name: &str) -> (Actor, sct_core::auth::IssuedCredential) {
    let identity = name.parse().unwrap();
    w.create_principal(admin, &identity).await.unwrap();
    let token = w.issue_token(admin, &identity).await.unwrap();
    let actor = w.authenticate(&token.secret, false, None).await.unwrap();
    (actor, token)
}
fn page() -> ListRequest {
    ListRequest {
        limit: 1.try_into().unwrap(),
        cursor: None,
    }
}
async fn collection(w: &Writer, a: &Actor, key: &str) -> ArchiveCollection {
    w.create_archive_collection(
        a,
        &CreateArchiveCollectionRequest {
            request_id: key.parse().unwrap(),
            label: "Archive".parse().unwrap(),
            media_capacity_bytes: 25_000_000_000.try_into().unwrap(),
        },
    )
    .await
    .unwrap()
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_management_isolates_accounts_and_rechecks_every_source() {
    let (w, admin, db) = setup().await;
    let a = root(&w, &admin, "/a", 500).await;
    let b = root(&w, &admin, "/b", 500).await;
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
    let mut selections = Vec::new();
    for (root, path) in [(&a, "first"), (&b, "second")] {
        let upload = ready_bytes(&engine, &w, &admin, &root.id, path, b"abc")
            .await
            .unwrap();
        let receipt = engine
            .commit_upload(&admin, &root.id, &upload)
            .await
            .unwrap();
        selections.push(ArchiveSelection {
            root_id: root.id.clone(),
            version_id: receipt.version_id,
            path: format!("/{path}").parse().unwrap(),
        });
    }
    let session = w.browser_session(&admin).await.unwrap();
    let browser = w.authenticate(&session.secret, true, None).await.unwrap();
    assert_eq!(
        ready_bytes(&engine, &w, &browser, &a.id, "denied", b"blocked")
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    let visible = w
        .resolve_path(&browser, &a.id, &"/first".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        w.hide_entry(
            &browser,
            &a.id,
            &visible.id,
            &HideEntryRequest {
                idempotency_key: "denied-hide".parse().unwrap(),
                expected_revision: visible.revision,
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    assert!(w.account(&browser).await.unwrap().administrator);
    let (owner, _) = user(&w, &admin, "owner").await;
    let (stranger, _) = user(&w, &admin, "stranger").await;
    let col = collection(&w, &owner, "first").await;
    collection(&w, &owner, "second").await;
    let mut request = CreateArchivePlanRequest {
        request_id: "capture".parse().unwrap(),
        expected_revision: col.revision.clone(),
        selections,
        media_capacity_bytes: col.media_capacity_bytes,
    };
    assert!(
        w.create_archive_plan(&owner, &col.id, &request)
            .await
            .is_err(),
        "collection ownership does not authorize roots"
    );
    let mut grants = Vec::new();
    for root in [&a, &b] {
        grants.push(
            w.create_grant(
                &admin,
                &CreateGrantRequest {
                    root_id: root.id.clone(),
                    principal_id: "owner".parse().unwrap(),
                    capabilities: vec![Capability::ListRead],
                },
            )
            .await
            .unwrap(),
        );
    }
    let set = w
        .create_archive_plan(&owner, &col.id, &request)
        .await
        .unwrap();
    assert!(w.authorize_archive_work(&set.id).await.unwrap());
    assert!(
        w.list_archive_collections(&stranger, &page())
            .await
            .unwrap()
            .collections
            .is_empty()
    );
    assert!(
        w.inspect_archive_collection(&stranger, &col.id)
            .await
            .is_err()
    );
    assert!(
        w.inspect_archive_set(&stranger, &col.id, &set.id)
            .await
            .is_err()
    );
    assert!(
        w.create_archive_plan(&stranger, &col.id, &request)
            .await
            .is_err()
    );
    let first = w.list_archive_collections(&owner, &page()).await.unwrap();
    let cursor = first.next_cursor.unwrap();
    assert!(
        w.list_archive_collections(
            &stranger,
            &ListRequest {
                cursor: Some(cursor.clone()),
                ..page()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        w.list_archive_collections(
            &owner,
            &ListRequest {
                cursor: Some(cursor),
                ..page()
            }
        )
        .await
        .unwrap()
        .collections
        .len(),
        1
    );
    sqlx::query("UPDATE archive_sets SET state='planning' WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    let mut tx = db.begin().await.unwrap();
    reserve_disc_numbers(&mut tx, &col.id, &set.id, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("INSERT INTO archive_discs VALUES('disc',$1,$2,1,repeat('a',64),repeat('b',64),repeat('c',64),0,0,100000000)").bind(col.id.as_str()).bind(set.id.as_str()).execute(&db).await.unwrap();
    let disc = "disc".parse().unwrap();
    assert!(
        w.authorize_archive_disc(&owner, &col.id, &disc)
            .await
            .is_ok()
    );
    assert!(
        w.authorize_archive_disc(&stranger, &col.id, &disc)
            .await
            .is_err()
    );
    w.revoke_grant(&admin, &grants[1].id, &grants[1].revision)
        .await
        .unwrap();
    assert!(
        w.inspect_archive_set(&owner, &col.id, &set.id)
            .await
            .is_err()
    );
    assert!(w.list_archive_sets(&owner, &col.id, &page()).await.is_err());
    assert!(
        w.list_archive_discs(&owner, &col.id, &set.id, &page())
            .await
            .is_err()
    );
    assert!(
        w.authorize_archive_disc(&owner, &col.id, &disc)
            .await
            .is_err()
    );
    assert!(!w.authorize_archive_work(&set.id).await.unwrap());
    assert!(w.cleanup_archive_capture(&set.id).await.unwrap());
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_sources s JOIN graph_pins p ON p.root_id=s.root_id AND p.id=s.pin_id WHERE s.set_id=$1").bind(set.id.as_str()).fetch_one(&db).await.unwrap(), 0);
    assert_eq!(
        w.inspect_archive_set(&admin, &col.id, &set.id)
            .await
            .unwrap()
            .state,
        ArchiveJobState::Revoked
    );
    request.request_id = "retry".parse().unwrap();
    assert!(
        w.create_archive_plan(&owner, &col.id, &request)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM management_audit WHERE event='archive.authority.revoked'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        1
    );
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_management_revisions_cancel_replay_and_credential_revocation() {
    let (w, admin, db) = setup().await;
    let r = root(&w, &admin, "/archive", 500).await;
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
    let upload = ready_bytes(&engine, &w, &admin, &r.id, "source", b"")
        .await
        .unwrap();
    let version = engine
        .commit_upload(&admin, &r.id, &upload)
        .await
        .unwrap()
        .version_id;
    engine.tick().await.unwrap();
    let (owner, credential) = user(&w, &admin, "owner").await;
    w.create_grant(
        &admin,
        &CreateGrantRequest {
            root_id: r.id.clone(),
            principal_id: "owner".parse().unwrap(),
            capabilities: vec![Capability::ListRead],
        },
    )
    .await
    .unwrap();
    let col = collection(&w, &owner, "collection").await;
    let update = UpdateArchiveCollectionRequest {
        expected_revision: col.revision,
        label: "Renamed".parse().unwrap(),
        media_capacity_bytes: 50_000_000_000.try_into().unwrap(),
    };
    let col = w
        .update_archive_collection(&owner, &col.id, &update)
        .await
        .unwrap();
    assert!(
        w.update_archive_collection(&owner, &col.id, &update)
            .await
            .is_err()
    );
    let request = CreateArchivePlanRequest {
        request_id: "capture".parse().unwrap(),
        expected_revision: col.revision,
        selections: vec![ArchiveSelection {
            root_id: r.id,
            version_id: version,
            path: "/source".parse().unwrap(),
        }],
        media_capacity_bytes: col.media_capacity_bytes,
    };
    let set = w
        .create_archive_plan(&owner, &col.id, &request)
        .await
        .unwrap();
    let action = ArchiveSetActionRequest {
        expected_revision: set.revision,
        request_id: "cancel".parse().unwrap(),
    };
    let cancelled = w
        .cancel_archive_set(&owner, &col.id, &set.id, &action)
        .await
        .unwrap();
    assert_eq!(cancelled.state, ArchiveJobState::Cancelled);
    assert_eq!(
        cancelled,
        w.cancel_archive_set(&owner, &col.id, &set.id, &action)
            .await
            .unwrap()
    );
    let col = w.inspect_archive_collection(&owner, &col.id).await.unwrap();
    let mut request = request;
    request.expected_revision = col.revision;
    request.request_id = "capture-two".parse().unwrap();
    let set = w
        .create_archive_plan(&owner, &col.id, &request)
        .await
        .unwrap();
    w.revoke_credential(&admin, &credential.id).await.unwrap();
    assert!(!w.authorize_archive_work(&set.id).await.unwrap());
    assert!(w.list_archive_collections(&owner, &page()).await.is_err());
    assert!(w.cleanup_archive_capture(&set.id).await.unwrap());
    assert!(w.cleanup_archive_capture(&cancelled.id).await.unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM graph_pins")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM archive_sets WHERE id=$1")
            .bind(set.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        "revoked"
    );
}
