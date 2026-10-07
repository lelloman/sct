use super::archive_planning_tests::{capture, collection, file, finish, publish_fixture, source};
use super::*;
fn page() -> ListRequest {
    ListRequest {
        limit: 100.try_into().unwrap(),
        cursor: None,
    }
}
fn observation(
    c: &ArchiveCollection,
    disc: &ArchiveLogicalDisc,
    copy: &ArchivePhysicalCopy,
) -> SubmitArchiveVerificationRequest {
    SubmitArchiveVerificationRequest {
        request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        expected_revision: copy.revision.clone(),
        collection_id: c.id.clone(),
        disc_id: disc.id.clone(),
        index_sha256: disc.index_sha256.clone(),
        bundle_sha256: disc.bundle_sha256.clone(),
        payload_inventory_sha256: disc.payload_inventory_sha256.clone(),
        expected_objects: disc.object_count,
        verified_objects: disc.object_count,
        verified_bytes: disc.payload_bytes,
        metadata_verified: true,
        verifier_version: "test-observation".parse().unwrap(),
    }
}
async fn register(
    w: &Writer,
    a: &Actor,
    c: &ArchiveCollection,
    d: &ArchiveLogicalDisc,
    name: &str,
) -> ArchivePhysicalCopy {
    let request = RegisterArchiveCopyRequest {
        request_id: name.parse().unwrap(),
        label: name.parse().unwrap(),
        location: "Shelf A / drawer 3".into(),
        reported_burned_at_ms: Some(123456.try_into().unwrap()),
    };
    let first = w
        .register_archive_copy(a, &c.id, &d.id, &request)
        .await
        .unwrap();
    assert_eq!(
        first,
        w.register_archive_copy(a, &c.id, &d.id, &request)
            .await
            .unwrap()
    );
    first
}
async fn current(
    w: &Writer,
    a: &Actor,
    c: &ArchiveCollection,
    d: &ArchiveLogicalDisc,
    copy: &ArchivePhysicalCopy,
) -> ArchivePhysicalCopy {
    w.list_archive_copies(a, &c.id, &d.id, &page())
        .await
        .unwrap()
        .copies
        .into_iter()
        .find(|v| v.id == copy.id)
        .unwrap()
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn copies_require_bound_read_back_and_preserve_completion_after_damage() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/copies", 4000).await;
    let v = source(&w, &a, &r.id).await;
    let c = collection(&w, &a, 3_300_000).await;
    let s = capture(&w, &a, &c, &r.id, &v).await;
    let chunks = vec![vec![1; 1_000_000], vec![2; 1_000_000]];
    file(&db, &s, &r.id, &v, "/file", &chunks).await;
    let s = finish(&w, &a, &s).await;
    publish_fixture(&w, &a, &db, &s, false).await;
    let discs = w
        .list_archive_discs(&a, &c.id, &s.id, &page())
        .await
        .unwrap()
        .discs;
    assert_eq!(discs.len(), 2);
    let one = register(&w, &a, &c, &discs[0], "one").await;
    let two = register(&w, &a, &c, &discs[1], "two").await;
    assert_eq!(one.health, ArchiveCopyHealth::Unknown);
    assert!(
        w.inspect_archive_set(&a, &c.id, &s.id)
            .await
            .unwrap()
            .verified_complete_at_ms
            .is_none()
    );
    let mut wrong = observation(&c, &discs[0], &one);
    wrong.payload_inventory_sha256 = "f".repeat(64).parse().unwrap();
    assert_eq!(
        w.submit_archive_verification(&a, &c.id, &one.id, &wrong)
            .await
            .unwrap_err()
            .0,
        ErrorCode::IntegrityFailure
    );
    wrong = observation(&c, &discs[0], &one);
    wrong.disc_id = discs[1].id.clone();
    assert_eq!(
        w.submit_archive_verification(&a, &c.id, &one.id, &wrong)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidRequest
    );
    let mut partial = observation(&c, &discs[0], &one);
    partial.verified_objects = 0.try_into().unwrap();
    partial.verified_bytes = 0.try_into().unwrap();
    let bad = w
        .submit_archive_verification(&a, &c.id, &one.id, &partial)
        .await
        .unwrap();
    assert!(!bad.complete_read_back);
    assert_eq!(
        bad,
        w.submit_archive_verification(&a, &c.id, &one.id, &partial)
            .await
            .unwrap()
    );
    let one = current(&w, &a, &c, &discs[0], &one).await;
    assert_eq!(one.health, ArchiveCopyHealth::Damaged);
    let good_request = observation(&c, &discs[0], &one);
    let good = w
        .submit_archive_verification(&a, &c.id, &one.id, &good_request)
        .await
        .unwrap();
    assert!(good.complete_read_back);
    assert!(
        w.inspect_archive_set(&a, &c.id, &s.id)
            .await
            .unwrap()
            .verified_complete_at_ms
            .is_none()
    );
    let mut metadata = observation(&c, &discs[1], &two);
    metadata.metadata_verified = false;
    assert!(
        !w.submit_archive_verification(&a, &c.id, &two.id, &metadata)
            .await
            .unwrap()
            .complete_read_back
    );
    let two = current(&w, &a, &c, &discs[1], &two).await;
    assert!(
        w.submit_archive_verification(&a, &c.id, &two.id, &observation(&c, &discs[1], &two))
            .await
            .unwrap()
            .complete_read_back
    );
    let completed = w
        .inspect_archive_set(&a, &c.id, &s.id)
        .await
        .unwrap()
        .verified_complete_at_ms
        .unwrap();
    let one = current(&w, &a, &c, &discs[0], &one).await;
    let one = w
        .update_archive_copy(
            &a,
            &c.id,
            &one.id,
            &UpdateArchiveCopyRequest {
                expected_revision: one.revision.clone(),
                label: one.label.clone(),
                location: "Updated shelf".into(),
                reported_burned_at_ms: None,
                health: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        one.health,
        ArchiveCopyHealth::ReadBackVerified,
        "label/location edits preserve verified health"
    );
    let mut request = UpdateArchiveCopyRequest {
        expected_revision: one.revision.clone(),
        label: one.label.clone(),
        location: "Offsite".into(),
        reported_burned_at_ms: None,
        health: Some(ArchiveCopyHealth::ReadBackVerified),
    };
    assert!(
        w.update_archive_copy(&a, &c.id, &one.id, &request)
            .await
            .is_err()
    );
    request.health = Some(ArchiveCopyHealth::Missing);
    let missing = w
        .update_archive_copy(&a, &c.id, &one.id, &request)
        .await
        .unwrap();
    assert_eq!(missing.health, ArchiveCopyHealth::Missing);
    assert_eq!(
        good,
        w.submit_archive_verification(&a, &c.id, &one.id, &good_request)
            .await
            .unwrap()
    );
    assert_eq!(
        current(&w, &a, &c, &discs[0], &one).await.health,
        ArchiveCopyHealth::Missing,
        "replaying an old report cannot undo later damage/loss"
    );
    assert_eq!(
        w.inspect_archive_set(&a, &c.id, &s.id)
            .await
            .unwrap()
            .verified_complete_at_ms,
        Some(completed)
    );
    assert_eq!(
        w.list_archive_reports(&a, &c.id, &one.id, &page())
            .await
            .unwrap()
            .reports
            .len(),
        2
    );
    let duplicate = register(&w, &a, &c, &discs[0], "duplicate").await;
    w.submit_archive_verification(
        &a,
        &c.id,
        &duplicate.id,
        &observation(&c, &discs[0], &duplicate),
    )
    .await
    .unwrap();
    let next = capture(&w, &a, &c, &r.id, &v).await;
    file(&db, &next, &r.id, &v, "/renamed", &chunks).await;
    let next = finish(&w, &a, &next).await;
    assert_eq!(
        next.plan.as_ref().unwrap().reused_payload_bytes.get(),
        2_000_000
    );
    publish_fixture(&w, &a, &db, &next, false).await;
    let next_discs = w
        .list_archive_discs(&a, &c.id, &next.id, &page())
        .await
        .unwrap()
        .discs;
    let new = next_discs.iter().find(|d| d.set_id == next.id).unwrap();
    let two = current(&w, &a, &c, &discs[1], &two).await;
    let two = w
        .update_archive_copy(
            &a,
            &c.id,
            &two.id,
            &UpdateArchiveCopyRequest {
                expected_revision: two.revision.clone(),
                label: two.label.clone(),
                location: two.location.clone(),
                reported_burned_at_ms: None,
                health: Some(ArchiveCopyHealth::Missing),
            },
        )
        .await
        .unwrap();
    let new_copy = register(&w, &a, &c, new, "new").await;
    w.submit_archive_verification(&a, &c.id, &new_copy.id, &observation(&c, new, &new_copy))
        .await
        .unwrap();
    assert!(
        w.inspect_archive_set(&a, &c.id, &next.id)
            .await
            .unwrap()
            .verified_complete_at_ms
            .is_none(),
        "a missing dependency prevents completion"
    );
    w.submit_archive_verification(&a, &c.id, &two.id, &observation(&c, &discs[1], &two))
        .await
        .unwrap();
    assert!(
        w.inspect_archive_set(&a, &c.id, &next.id)
            .await
            .unwrap()
            .verified_complete_at_ms
            .is_some()
    );
    let events = w
        .list_archive_copy_events(&a, &c.id, &one.id, &page())
        .await
        .unwrap()
        .events;
    assert_eq!(events.len(), 5);
    assert!(
        events
            .iter()
            .any(|e| e.kind == ArchiveCopyEventKind::ReadBack
                && e.observation.health == ArchiveCopyHealth::ReadBackVerified)
    );
    w.create_principal(&a, &"stranger".parse().unwrap())
        .await
        .unwrap();
    let credential = w
        .issue_token(&a, &"stranger".parse().unwrap())
        .await
        .unwrap();
    let stranger = w
        .authenticate(&credential.secret, false, None)
        .await
        .unwrap();
    assert_eq!(
        w.list_archive_copies(&stranger, &c.id, &discs[0].id, &page())
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.list_archive_reports(&stranger, &c.id, &one.id, &page())
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert_eq!(
        w.submit_archive_verification(&stranger, &c.id, &one.id, &good_request)
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    assert!(
        sqlx::query("DELETE FROM archive_copy_events")
            .execute(&db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM archive_reports")
            .execute(&db)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_discs")
            .fetch_one(&db)
            .await
            .unwrap(),
        3
    );
}
