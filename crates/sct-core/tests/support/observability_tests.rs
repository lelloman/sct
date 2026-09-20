use super::*;
use sct_core::{
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn overdue_tracks_only_successful_publication_and_replay_does_not_refresh_it() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/freshness", 4000).await;
    let initial = w.root_management(&a, &r.id).await.unwrap();
    assert_eq!(initial.backup_freshness, BackupFreshness::NotConfigured);
    let managed = w
        .update_root_management(
            &a,
            &r.id,
            &UpdateRootManagementRequest {
                expected_revision: initial.revision,
                display_name: r.display_name.clone(),
                quota_bytes: r.quota_bytes,
                expected_upload_interval_seconds: Some(60.try_into().unwrap()),
            },
        )
        .await
        .unwrap();
    assert_eq!(managed.backup_freshness, BackupFreshness::NeverCommitted);
    assert!(managed.next_expected_at_ms.is_none());
    let e = TransferEngine::new(
        w.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        3600,
    )
    .unwrap();
    let upload = ready_bytes(&e, &w, &a, &r.id, "first", b"saved")
        .await
        .unwrap();
    assert_eq!(
        w.root_management(&a, &r.id).await.unwrap().backup_freshness,
        BackupFreshness::NeverCommitted
    );
    e.commit_upload(&a, &r.id, &upload).await.unwrap();
    e.tick().await.unwrap();
    let committed = w.root_management(&a, &r.id).await.unwrap();
    assert_eq!(committed.backup_freshness, BackupFreshness::OnTime);
    assert_eq!(
        committed.next_expected_at_ms.unwrap().get() - committed.last_commit_at_ms.unwrap().get(),
        60000
    );
    sqlx::query(
        "UPDATE roots SET last_commit_at=clock_timestamp()-interval '2 minutes' WHERE id=$1",
    )
    .bind(r.id.as_str())
    .execute(&db)
    .await
    .unwrap();
    let stale = w.root_management(&a, &r.id).await.unwrap();
    assert_eq!(stale.backup_freshness, BackupFreshness::Overdue);
    e.commit_upload(&a, &r.id, &upload).await.unwrap();
    assert_eq!(
        w.root_management(&a, &r.id)
            .await
            .unwrap()
            .last_commit_at_ms,
        stale.last_commit_at_ms
    );
    let attempt = ready_bytes(&e, &w, &a, &r.id, "second", b"not committed")
        .await
        .unwrap();
    e.abort_upload(&a, &r.id, &attempt).await.unwrap();
    assert_eq!(
        w.root_management(&a, &r.id)
            .await
            .unwrap()
            .last_commit_at_ms,
        stale.last_commit_at_ms
    );
    let metrics = w.service_metrics(&a).await.unwrap();
    assert!(metrics.freshness.contains(&("overdue".into(), 1)));
    let text = serde_json::to_string(&metrics.capacity).unwrap();
    assert!(!text.contains(r.id.as_str()));
    // Migration backfill uses committed publication audit, never admission time.
    sqlx::query("UPDATE roots SET last_commit_at=NULL WHERE id=$1")
        .bind(r.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0018_commit_freshness.sql"))
        .execute(&db)
        .await
        .unwrap();
    assert!(
        w.root_management(&a, &r.id)
            .await
            .unwrap()
            .last_commit_at_ms
            .is_some()
    );
}
