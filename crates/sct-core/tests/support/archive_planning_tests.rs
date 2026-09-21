use super::*;
use sct_core::{
    archive_planning::ArchiveIndexCursor,
    payload::{MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use sqlx::Row;
use std::collections::BTreeMap;
fn bundle() -> ArchiveBundle {
    serde_json::from_slice(include_bytes!(
        "../../../../contracts/fixtures/disc-archive/disc-1/recovery/bundle.json"
    ))
    .unwrap()
}
async fn source(w: &Writer, a: &Actor, r: &RootId) -> VersionId {
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
    let parent = w.resolve_path(a, r, &"/".parse().unwrap()).await.unwrap();
    let directory = mkdir(
        w,
        a,
        r,
        &parent.id,
        "anchor",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let request = super::tree_draft_tests::draft_request(
        w,
        a,
        r,
        &directory.id,
        DraftMode::CompleteReplacement,
    )
    .await;
    let u = e.create_tree_draft(a, r, &request).await.unwrap().id;
    e.freeze_tree_draft(a, r, &u).await.unwrap();
    super::tree_build_tests::admitted(&e, a, r, &u).await;
    super::tree_publication_tests::seal(&e, a, r, &u).await;
    let version = e.commit_upload(a, r, &u).await.unwrap().version_id;
    e.tick().await.unwrap();
    version
}
async fn collection(w: &Writer, a: &Actor, capacity: u64) -> ArchiveCollection {
    w.create_archive_collection(
        a,
        &CreateArchiveCollectionRequest {
            request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
            label: "Planner".parse().unwrap(),
            media_capacity_bytes: capacity.try_into().unwrap(),
        },
    )
    .await
    .unwrap()
}
async fn capture(
    w: &Writer,
    a: &Actor,
    c: &ArchiveCollection,
    r: &RootId,
    v: &VersionId,
) -> ArchiveSet {
    let c = w.inspect_archive_collection(a, &c.id).await.unwrap();
    let s = w
        .create_archive_plan(
            a,
            &c.id,
            &CreateArchivePlanRequest {
                request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
                expected_revision: c.revision,
                selections: vec![ArchiveSelection {
                    root_id: r.clone(),
                    version_id: v.clone(),
                    path: "/".parse().unwrap(),
                }],
                media_capacity_bytes: c.media_capacity_bytes,
            },
        )
        .await
        .unwrap();
    for _ in 0..20 {
        if !w.advance_archive_capture(&s.id).await.unwrap() {
            break;
        }
    }
    s
}
// Planner fixtures directly populate an already captured graph; source capture
// itself is exercised against real published trees by archive_capture_tests.
async fn file(
    db: &PgPool,
    s: &ArchiveSet,
    r: &RootId,
    v: &VersionId,
    path: &str,
    chunks: &[Vec<u8>],
) {
    let length = chunks.iter().map(|c| c.len() as i64).sum::<i64>();
    sqlx::query("INSERT INTO archive_entries(set_id,path,kind,root_id,version_id,length) VALUES($1,$2,'file',$3,$4,$5)").bind(s.id.as_str()).bind(path).bind(r.as_str()).bind(v.as_str()).bind(length).execute(db).await.unwrap();
    let mut offset = 0;
    for bytes in chunks {
        let hash = digest(bytes);
        sqlx::query("INSERT INTO archive_objects(set_id,root_id,digest,length) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(s.id.as_str()).bind(r.as_str()).bind(&hash).bind(bytes.len() as i64).execute(db).await.unwrap();
        sqlx::query("INSERT INTO archive_extents(set_id,path,offset_bytes,root_id,digest,length) VALUES($1,$2,$3,$4,$5,$6)").bind(s.id.as_str()).bind(path).bind(offset).bind(r.as_str()).bind(hash).bind(bytes.len() as i64).execute(db).await.unwrap();
        offset += bytes.len() as i64;
    }
    sqlx::query("UPDATE archive_sets SET processed_entries=processed_entries+1,processed_bytes=processed_bytes+$2 WHERE id=$1").bind(s.id.as_str()).bind(length).execute(db).await.unwrap();
}
async fn finish(w: &Writer, a: &Actor, s: &ArchiveSet) -> ArchiveSet {
    for i in 0..1000 {
        if i % 16 == 0 {
            w.renew().await.unwrap();
        }
        if !w.advance_archive_plan(&s.id, &bundle()).await.unwrap() {
            break;
        }
    }
    let result = w
        .inspect_archive_set(a, &s.collection_id, &s.id)
        .await
        .unwrap();
    assert_eq!(result.state, ArchiveJobState::AwaitingConfirmation);
    result
}
async fn lines(w: &Writer, s: &ArchiveSet) -> Vec<u8> {
    let mut cursor = ArchiveIndexCursor::default();
    let mut all = Vec::new();
    loop {
        let page = w.archive_index_page(&s.id, &cursor).await.unwrap();
        if page.is_empty() {
            break;
        }
        for (c, b) in page {
            cursor = c;
            all.extend(b);
        }
    }
    all
}
async fn publish_fixture(w: &Writer, a: &Actor, db: &PgPool, s: &ArchiveSet, verified: bool) {
    w.confirm_archive_set(
        a,
        &s.collection_id,
        &s.id,
        &ArchiveSetActionRequest {
            request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
            expected_revision: s.revision.clone(),
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO archive_artifacts(id,disc_id,state,sha256,length,relative_path,expires_at) SELECT 'artifact_'||id,id,'available',repeat('a',64),planned_iso_bytes,id||'.iso',clock_timestamp()+interval '1 day' FROM archive_discs WHERE set_id=$1").bind(s.id.as_str()).execute(db).await.unwrap();
    sqlx::query("UPDATE archive_sets SET state='ready',published_at=clock_timestamp() WHERE id=$1")
        .bind(s.id.as_str())
        .execute(db)
        .await
        .unwrap();
    if verified {
        sqlx::query("UPDATE archive_sets SET verified_complete_at=clock_timestamp() WHERE id=$1")
            .bind(s.id.as_str())
            .execute(db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO archive_copies(id,disc_id,label,revision,health) SELECT 'copy_'||id,id,'fixture','r1','read_back_verified' FROM archive_discs WHERE set_id=$1").bind(s.id.as_str()).execute(db).await.unwrap();
    }
    w.cleanup_archive_capture(&s.id).await.unwrap();
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_planner_restart_multidisc_locality_and_independent_reader() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/planner", 4000).await;
    let v = source(&w, &a, &r.id).await;
    let c = collection(&w, &a, 3_300_000).await;
    let s = capture(&w, &a, &c, &r.id, &v).await;
    let chunks = vec![
        vec![1; 1_000_000],
        vec![2; 1_000_000],
        vec![3; 1_000_000],
        vec![4; 1_000_000],
    ];
    file(&db, &s, &r.id, &v, "/a-large", &chunks).await;
    file(&db, &s, &r.id, &v, "/z-shared", &chunks[..1]).await;
    file(&db, &s, &r.id, &v, "/é-empty", &[]).await;
    for _ in 0..4 {
        w.advance_archive_plan(&s.id, &bundle()).await.unwrap();
    }
    w.release().await.unwrap();
    let next = w.catalog().acquire_writer().await.unwrap();
    assert!(w.advance_archive_plan(&s.id, &bundle()).await.is_err());
    let result = finish(&next, &a, &s).await;
    let plan = result.plan.as_ref().unwrap();
    assert_eq!(plan.new_disc_count.get(), 4);
    assert_eq!(plan.new_payload_bytes.get(), 4_000_000);
    assert_eq!(plan.source_bytes.get(), 5_000_000);
    let index = lines(&next, &s).await;
    let parsed: Vec<_> = index
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| archive_record(l).unwrap())
        .collect();
    let filenames: Vec<_> = parsed
        .iter()
        .filter_map(|r| {
            if let ArchiveRecord::File { path, .. } = r {
                Some(path.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(filenames, vec!["/a-large", "/z-shared", "/é-empty"]);
    let rows=sqlx::query("SELECT id,planned_iso_bytes,object_count FROM archive_discs WHERE set_id=$1 ORDER BY number").bind(s.id.as_str()).fetch_all(&db).await.unwrap();
    let scratch = std::env::temp_dir().join(format!("sct-planner-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&scratch).unwrap();
    for (number, row) in rows.iter().enumerate() {
        assert!(row.get::<i64, _>("planned_iso_bytes") <= 3_300_000);
        assert_eq!(row.get::<i64, _>("object_count"), 1);
        let disc: ArchiveDiscId = row.get::<String, _>("id").parse().unwrap();
        let descriptor = next.archive_disc_descriptor(&s.id, &disc).await.unwrap();
        assert_eq!(descriptor.index_sha256.as_str(), digest(&index));
        let media = scratch.join(number.to_string());
        std::fs::create_dir_all(media.join("recovery")).unwrap();
        std::fs::write(media.join("disc.json"), canonical(&descriptor).unwrap()).unwrap();
        std::fs::write(media.join("snapshot.jsonl"), &index).unwrap();
        std::fs::write(
            media.join("recovery/bundle.json"),
            canonical(&bundle()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            media.join("START-HERE.txt"),
            include_bytes!("../../../../contracts/fixtures/disc-archive/disc-1/START-HERE.txt"),
        )
        .unwrap();
        let obj = ArchiveObject {
            root_id: r.id.clone(),
            digest: digest(&chunks[number]).parse().unwrap(),
            length: 1_000_000.try_into().unwrap(),
        };
        let path = media.join(obj.relative_path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &chunks[number]).unwrap();
        let mut reader =
            sct_archive::ArchiveIndex::create(&media, &scratch.join(format!("{number}.sqlite")))
                .await
                .unwrap();
        assert!(
            reader
                .verify_media(&media, &mut Vec::new())
                .await
                .unwrap()
                .complete()
        );
    }
    std::fs::remove_dir_all(scratch).unwrap();
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_planner_reuses_only_verified_healthy_same_root_and_never_rewrites_history() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/reuse", 4000).await;
    let v = source(&w, &a, &r.id).await;
    let c = collection(&w, &a, 4_000_000).await;
    let bytes = vec![vec![7; 1000]];
    let first = capture(&w, &a, &c, &r.id, &v).await;
    file(&db, &first, &r.id, &v, "/old-name", &bytes).await;
    let first = finish(&w, &a, &first).await;
    let original = lines(&w, &first).await;
    publish_fixture(&w, &a, &db, &first, false).await;
    assert!(
        sqlx::query("UPDATE archive_index_records SET line=line WHERE set_id=$1")
            .bind(first.id.as_str())
            .execute(&db)
            .await
            .is_err()
    );
    let second = capture(&w, &a, &c, &r.id, &v).await;
    file(&db, &second, &r.id, &v, "/renamed", &bytes).await;
    let second = finish(&w, &a, &second).await;
    assert_eq!(second.plan.as_ref().unwrap().reused_payload_bytes.get(), 0);
    publish_fixture(&w, &a, &db, &second, true).await;
    let third = capture(&w, &a, &c, &r.id, &v).await;
    file(&db, &third, &r.id, &v, "/renamed-again", &bytes).await;
    let third = finish(&w, &a, &third).await;
    let p = third.plan.as_ref().unwrap();
    assert_eq!(p.new_payload_bytes.get(), 0);
    assert_eq!(p.reused_payload_bytes.get(), 1000);
    assert_eq!(p.new_disc_count.get(), 1);
    assert_eq!(p.required_earlier_disc_count.get(), 1);
    let before: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT line FROM archive_index_records WHERE set_id=$1 ORDER BY phase,key1,key2,position",
    )
    .bind(first.id.as_str())
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(before.concat(), original);
    sqlx::query("UPDATE archive_copies SET health='damaged'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        w.confirm_archive_set(
            &a,
            &c.id,
            &third.id,
            &ArchiveSetActionRequest {
                request_id: "stale-media".parse().unwrap(),
                expected_revision: third.revision.clone()
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::ArchiveMediaUnavailable
    );
    sqlx::query("UPDATE archive_copies SET health='read_back_verified'")
        .execute(&db)
        .await
        .unwrap();
    w.cancel_archive_set(
        &a,
        &c.id,
        &third.id,
        &ArchiveSetActionRequest {
            request_id: "cancel-third".parse().unwrap(),
            expected_revision: third.revision,
        },
    )
    .await
    .unwrap();
    w.cleanup_archive_capture(&third.id).await.unwrap();
    sqlx::query("UPDATE archive_copies SET health='damaged'")
        .execute(&db)
        .await
        .unwrap();
    let fourth = capture(&w, &a, &c, &r.id, &v).await;
    file(&db, &fourth, &r.id, &v, "/fourth", &bytes).await;
    let fourth = finish(&w, &a, &fourth).await;
    assert_eq!(fourth.plan.as_ref().unwrap().reused_payload_bytes.get(), 0);
    publish_fixture(&w, &a, &db, &fourth, false).await;
    sqlx::query("UPDATE archive_copies SET health='read_back_verified'")
        .execute(&db)
        .await
        .unwrap();
    let other = root(&w, &a, "/other", 4000).await;
    let other_version = source(&w, &a, &other.id).await;
    let fifth = capture(&w, &a, &c, &other.id, &other_version).await;
    file(
        &db,
        &fifth,
        &other.id,
        &other_version,
        "/same-bytes-other-root",
        &bytes,
    )
    .await;
    let fifth = finish(&w, &a, &fifth).await;
    assert_eq!(fifth.plan.unwrap().reused_payload_bytes.get(), 0);
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_planner_empty_snapshot_bundle_identity_and_capacity_failures() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/limits", 4000).await;
    let v = source(&w, &a, &r.id).await;
    let c = collection(&w, &a, 4_000_000).await;
    let s = capture(&w, &a, &c, &r.id, &v).await;
    w.advance_archive_plan(&s.id, &bundle()).await.unwrap();
    let mut changed = bundle();
    changed.version = "changed".into();
    assert_eq!(
        w.advance_archive_plan(&s.id, &changed).await.unwrap_err().0,
        ErrorCode::IntegrityFailure
    );
    let result = finish(&w, &a, &s).await;
    assert_eq!(result.plan.as_ref().unwrap().new_disc_count.get(), 1);
    assert_eq!(result.plan.unwrap().new_payload_bytes.get(), 0);
    let tiny = collection(&w, &a, 2_000_000).await;
    let s = capture(&w, &a, &tiny, &r.id, &v).await;
    let mut failure = None;
    for _ in 0..20 {
        if let Err(e) = w.advance_archive_plan(&s.id, &bundle()).await {
            failure = Some(e.0);
            break;
        }
    }
    assert_eq!(failure, Some(ErrorCode::ArchiveMetadataTooLarge));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT next_disc FROM archive_collections WHERE id=$1")
            .bind(tiny.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_planner_exact_budget_boundary_and_permanent_cancelled_numbers() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/boundary", 4000).await;
    let v = source(&w, &a, &r.id).await;
    let c = collection(&w, &a, 4_000_000).await;
    let s = capture(&w, &a, &c, &r.id, &v).await;
    let chunks = vec![vec![9; 4096]];
    file(&db, &s, &r.id, &v, "/boundary", &chunks).await;
    let s = finish(&w, &a, &s).await;
    let budget =
        sqlx::query_scalar::<_, i64>("SELECT planned_iso_bytes FROM archive_discs WHERE set_id=$1")
            .bind(s.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap() as u64;
    w.cancel_archive_set(
        &a,
        &c.id,
        &s.id,
        &ArchiveSetActionRequest {
            request_id: "cancel".parse().unwrap(),
            expected_revision: s.revision,
        },
    )
    .await
    .unwrap();
    w.cleanup_archive_capture(&s.id).await.unwrap();
    let next = capture(&w, &a, &c, &r.id, &v).await;
    let next = finish(&w, &a, &next).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT number FROM archive_discs WHERE set_id=$1")
            .bind(next.id.as_str())
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    for capacity in [budget, budget - 1] {
        let c = collection(&w, &a, capacity).await;
        let s = capture(&w, &a, &c, &r.id, &v).await;
        file(&db, &s, &r.id, &v, "/boundary", &chunks).await;
        if capacity == budget {
            let s = finish(&w, &a, &s).await;
            assert_eq!(s.plan.unwrap().new_disc_count.get(), 1);
        } else {
            let mut error = None;
            for _ in 0..20 {
                if let Err(e) = w.advance_archive_plan(&s.id, &bundle()).await {
                    error = Some(e.0);
                    break;
                }
            }
            assert_eq!(error, Some(ErrorCode::ArchiveMetadataTooLarge));
        }
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_planner_does_not_reuse_disc_with_an_inaccessible_other_source() {
    let (w, admin, db) = setup().await;
    let a = root(&w, &admin, "/a", 4000).await;
    let b = root(&w, &admin, "/b", 4000).await;
    let av = source(&w, &admin, &a.id).await;
    let bv = source(&w, &admin, &b.id).await;
    w.create_principal(&admin, &"owner".parse().unwrap())
        .await
        .unwrap();
    let token = w
        .issue_token(&admin, &"owner".parse().unwrap())
        .await
        .unwrap();
    let user = w.authenticate(&token.secret, false, None).await.unwrap();
    let mut grants = Vec::new();
    for r in [&a, &b] {
        grants.push(
            w.create_grant(
                &admin,
                &CreateGrantRequest {
                    root_id: r.id.clone(),
                    principal_id: "owner".parse().unwrap(),
                    capabilities: vec![Capability::ListRead],
                },
            )
            .await
            .unwrap(),
        );
    }
    let c = collection(&w, &user, 4_000_000).await;
    let s = w
        .create_archive_plan(
            &user,
            &c.id,
            &CreateArchivePlanRequest {
                request_id: "mixed".parse().unwrap(),
                expected_revision: c.revision.clone(),
                selections: vec![
                    ArchiveSelection {
                        root_id: a.id.clone(),
                        version_id: av.clone(),
                        path: "/a".parse().unwrap(),
                    },
                    ArchiveSelection {
                        root_id: b.id.clone(),
                        version_id: bv,
                        path: "/b".parse().unwrap(),
                    },
                ],
                media_capacity_bytes: c.media_capacity_bytes,
            },
        )
        .await
        .unwrap();
    for _ in 0..20 {
        if !w.advance_archive_capture(&s.id).await.unwrap() {
            break;
        }
    }
    let bytes = vec![vec![3; 1000]];
    file(&db, &s, &a.id, &av, "/a/file", &bytes).await;
    let s = finish(&w, &user, &s).await;
    publish_fixture(&w, &user, &db, &s, true).await;
    w.revoke_grant(&admin, &grants[1].id, &grants[1].revision)
        .await
        .unwrap();
    let next = capture(&w, &user, &c, &a.id, &av).await;
    file(&db, &next, &a.id, &av, "/file", &bytes).await;
    let next = finish(&w, &user, &next).await;
    assert_eq!(next.plan.as_ref().unwrap().reused_payload_bytes.get(), 0);
    assert_eq!(next.plan.unwrap().new_payload_bytes.get(), 1000);
}
