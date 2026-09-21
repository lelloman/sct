use super::*;
use sct_core::archive_catalog::reserve_disc_numbers;
fn collection_request() -> CreateArchiveCollectionRequest {
    CreateArchiveCollectionRequest {
        request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        label: "Optical archive".parse().unwrap(),
        media_capacity_bytes: 25_000_000_000.try_into().unwrap(),
    }
}
async fn plan_request(
    writer: &Writer,
    actor: &Actor,
    collection: &ArchiveCollection,
    root: &RootInspection,
) -> CreateArchivePlanRequest {
    use sct_core::{
        payload::{MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    let engine = TransferEngine::new(
        writer.clone(),
        std::collections::BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        3600,
    )
    .unwrap();
    let upload = ready_bytes(&engine, writer, actor, &root.id, "capture-source", b"")
        .await
        .unwrap();
    let version = engine
        .commit_upload(actor, &root.id, &upload)
        .await
        .unwrap()
        .version_id;
    engine.tick().await.unwrap();
    CreateArchivePlanRequest {
        request_id: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
        expected_revision: collection.revision.clone(),
        selections: vec![ArchiveSelection {
            root_id: root.id.clone(),
            version_id: version,
            path: "/capture-source".parse().unwrap(),
        }],
        media_capacity_bytes: collection.media_capacity_bytes,
    }
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_catalog_serializes_append_replays_and_preserves_numbers_on_restart() {
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/archive", 500).await;
    let request = collection_request();
    let collection = writer
        .create_archive_collection(&actor, &request)
        .await
        .unwrap();
    assert_eq!(
        collection,
        writer
            .create_archive_collection(&actor, &request)
            .await
            .unwrap()
    );
    let mut changed = request.clone();
    changed.label = "different".parse().unwrap();
    assert!(matches!(
        writer
            .create_archive_collection(&actor, &changed)
            .await
            .unwrap_err()
            .0,
        ErrorCode::IdempotencyConflict
    ));
    let request = plan_request(&writer, &actor, &collection, &root).await;
    let (one, two) = tokio::join!(
        writer.create_archive_plan(&actor, &collection.id, &request),
        writer.create_archive_plan(&actor, &collection.id, &request)
    );
    let set = one.unwrap();
    assert_eq!(set, two.unwrap());
    let fresh_revision: String =
        sqlx::query_scalar("SELECT revision FROM archive_collections WHERE id=$1")
            .bind(collection.id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut other = request.clone();
    other.request_id = "another".parse().unwrap();
    other.expected_revision = fresh_revision.parse().unwrap();
    assert!(matches!(
        writer
            .create_archive_plan(&actor, &collection.id, &other)
            .await
            .unwrap_err()
            .0,
        ErrorCode::ArchiveBusy
    ));
    sqlx::query("UPDATE archive_sets SET state='planning' WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        reserve_disc_numbers(&mut tx, &collection.id, &set.id, 10)
            .await
            .unwrap(),
        (1, 10)
    );
    assert_eq!(
        reserve_disc_numbers(&mut tx, &collection.id, &set.id, 10)
            .await
            .unwrap(),
        (1, 10)
    );
    tx.commit().await.unwrap();
    sqlx::query("UPDATE archive_sets SET state='cancelled' WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("DELETE FROM archive_sets WHERE id=$1")
            .bind(set.id.as_str())
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE archive_collections SET next_disc=1 WHERE id=$1")
            .bind(collection.id.as_str())
            .execute(&pool)
            .await
            .is_err()
    );
    writer.release().await.unwrap();
    let restarted = writer.catalog().acquire_writer().await.unwrap();
    let revision: String =
        sqlx::query_scalar("SELECT revision FROM archive_collections WHERE id=$1")
            .bind(collection.id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    other.expected_revision = revision.parse().unwrap();
    let next = restarted
        .create_archive_plan(&actor, &collection.id, &other)
        .await
        .unwrap();
    assert_eq!(next.sequence.get(), 2);
    sqlx::query("UPDATE archive_sets SET state='planning' WHERE id=$1")
        .bind(next.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        reserve_disc_numbers(&mut tx, &collection.id, &next.id, 2)
            .await
            .unwrap(),
        (11, 12)
    );
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_sets")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_catalog_freezes_inventory_and_keeps_copy_observations() {
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/archive", 500).await;
    let collection = writer
        .create_archive_collection(&actor, &collection_request())
        .await
        .unwrap();
    let set = writer
        .create_archive_plan(
            &actor,
            &collection.id,
            &plan_request(&writer, &actor, &collection, &root).await,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE archive_sets SET state='planning' WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    reserve_disc_numbers(&mut tx, &collection.id, &set.id, 1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::query(
        "INSERT INTO archive_entries VALUES($1,'/','directory',NULL,NULL,0) ON CONFLICT DO NOTHING",
    )
    .bind(set.id.as_str())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO archive_discs VALUES('disc',$1,$2,1,repeat('a',64),repeat('b',64),repeat('c',64),0,0,100000000)").bind(collection.id.as_str()).bind(set.id.as_str()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO archive_artifacts(id,disc_id,state,sha256,length,relative_path,expires_at,download_count) VALUES('image','disc','building',NULL,NULL,NULL,clock_timestamp()+interval '1 day',0)").execute(&pool).await.unwrap();
    sqlx::query("UPDATE archive_sets SET state='awaiting_confirmation',plan='{}' WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE archive_sets SET state='building',frozen_at=clock_timestamp() WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    for query in [
        "UPDATE archive_entries SET path='/changed' WHERE set_id=$1",
        "DELETE FROM archive_entries WHERE set_id=$1",
        "INSERT INTO archive_entries VALUES($1,'/late','directory',NULL,NULL,0)",
        "UPDATE archive_sets SET plan='{\"changed\":true}' WHERE id=$1",
        "UPDATE archive_discs SET index_sha256=repeat('f',64) WHERE set_id=$1",
    ] {
        assert!(
            sqlx::query(query)
                .bind(set.id.as_str())
                .execute(&pool)
                .await
                .is_err(),
            "{query}"
        );
    }
    sqlx::query("UPDATE archive_artifacts SET state='available',sha256=repeat('d',64),length=4096,relative_path='image.iso' WHERE id='image'").execute(&pool).await.unwrap();
    assert!(
        sqlx::query("UPDATE archive_artifacts SET sha256=repeat('e',64) WHERE id='image'")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE archive_sets SET state='ready',published_at=clock_timestamp() WHERE id=$1")
        .bind(set.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    for copy in ["copy-a", "copy-b"] {
        sqlx::query(
            "INSERT INTO archive_copies(id,disc_id,label,revision) VALUES($1,'disc',$1,'r1')",
        )
        .bind(copy)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO archive_reports(id,copy_id,principal_id,observation,complete_read_back) VALUES('report','copy-a',$1,'{}',true)").bind(actor.principal_id()).execute(&pool).await.unwrap();
    sqlx::query("UPDATE archive_copies SET health='read_back_verified',latest_report_id='report' WHERE id='copy-a'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE archive_copies SET health='damaged' WHERE id='copy-a'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE archive_artifacts SET state='expired',relative_path=NULL WHERE id='image'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_reports")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(
        sqlx::query("DELETE FROM archive_reports")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM archive_copies")
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM archive_sets WHERE id=$1")
            .bind(set.id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap(),
        "ready"
    );
    let online_fks:i64=sqlx::query_scalar("SELECT count(*) FROM pg_constraint WHERE contype='f' AND conrelid::regclass::text LIKE 'archive_%' AND confrelid IN ('file_versions'::regclass,'roots'::regclass,'content_objects'::regclass)").fetch_one(&pool).await.unwrap();
    assert_eq!(online_fks, 0);
}
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn archive_catalog_migrates_existing_database_without_rewriting_online_history() {
    let (catalog, pool) = support::database_through(Some(19)).await;
    sqlx::query("INSERT INTO principals(id) VALUES('existing')")
        .execute(&pool)
        .await
        .unwrap();
    catalog.migrate().await.unwrap();
    catalog.migrate().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM principals WHERE id='existing'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        24
    );
}
