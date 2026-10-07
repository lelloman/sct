use super::*;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn migration_preserves_existing_m2_version_and_pin() {
    let (catalog, db) = support::database_through(Some(4)).await;
    sqlx::raw_sql("INSERT INTO principals(id) VALUES('p');
        INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision)
        VALUES('r','p','root','/','revision','enabled','{}',1000,'{}','policy','policy-revision');
        INSERT INTO entries(id,root_id,name,kind,revision) VALUES('e','r','root','directory','rev');
        INSERT INTO entries(id,root_id,parent_id,name,kind,revision) VALUES('f','r','e','file','file','rev');
        INSERT INTO content_objects(root_id,digest,kind,length) VALUES('r',repeat('a',64),'file',0);
        INSERT INTO file_versions(id,root_id,entry_id,manifest,length) VALUES('v','r','f',repeat('a',64),0);
        UPDATE entries SET current_version_id='v' WHERE id='f';
        INSERT INTO read_pins(id,root_id,version_id,principal_id,expires_at) VALUES('pin','r','v','p',clock_timestamp()+interval '1 hour');")
        .execute(&db).await.unwrap();
    catalog.migrate().await.unwrap();
    catalog.migrate().await.unwrap();
    let preserved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM file_versions v JOIN entries e ON e.current_version_id=v.id JOIN read_pins p ON p.version_id=v.id WHERE v.id='v' AND v.kind='file' AND NOT v.retired)")
        .fetch_one(&db).await.unwrap();
    assert!(preserved);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn snapshot_catalog_uses_immutable_roots_and_enforces_boundaries() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/trees", 4000).await;
    let other = root(&w, &a, "/other-trees", 4000).await;
    let boundary = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let empty = DirectoryManifest {
        format: 1,
        kind: DirectoryObjectKind::Directory,
        entries: 0.try_into().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
        height: 0,
        tree: None,
    };
    let empty_body = canonical(&empty).unwrap();
    let empty_id = digest(&empty_body);
    let expected = DirectoryEntry {
        id: "stable-child".parse().unwrap(),
        name: "empty".parse().unwrap(),
        kind: EntryKind::Directory,
        manifest: empty_id.parse().unwrap(),
        length: 0.try_into().unwrap(),
        chunks: 0.try_into().unwrap(),
    };
    let page = canonical(&DirectoryPage::DirectoryEntries {
        format: 1,
        entries: vec![expected.clone()],
    })
    .unwrap();
    let page_id = digest(&page);
    let tree = DirectoryManifest {
        entries: 1.try_into().unwrap(),
        tree: Some(page_id.parse().unwrap()),
        ..empty.clone()
    };
    let tree_body = canonical(&tree).unwrap();
    let tree_id = digest(&tree_body);
    for (id, kind, body) in [
        (&empty_id, "directory", &empty_body),
        (&page_id, "directory_page", &page),
        (&tree_id, "directory", &tree_body),
    ] {
        sqlx::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,$3,0,0,$4)")
            .bind(r.id.as_str()).bind(id).bind(kind).bind(body).execute(&db).await.unwrap();
    }
    let before: i64 = sqlx::query_scalar("SELECT mutation_serial FROM roots WHERE id=$1")
        .bind(r.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length,kind) VALUES('snapshot', $1,$2,$3,0,'directory')")
        .bind(r.id.as_str()).bind(boundary.id.as_str()).bind(&tree_id).execute(&db).await.unwrap();
    let version: VersionId = "snapshot".parse().unwrap();
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &version, &["empty".parse().unwrap()])
            .await
            .unwrap(),
        expected
    );
    assert_eq!(
        w.snapshot_entry(&a, &other.id, &version, &["empty".parse().unwrap()])
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    let descendants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entries WHERE root_id=$1 AND parent_id IS NOT NULL",
    )
    .bind(r.id.as_str())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        descendants, 0,
        "snapshots require no materialized namespace descendants"
    );
    assert!(sqlx::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length) VALUES('wrong-kind',$1,$2,$3,0)")
        .bind(r.id.as_str()).bind(boundary.id.as_str()).bind(&tree_id).execute(&db).await.is_err());
    assert!(sqlx::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length,kind) VALUES('wrong-root',$1,$2,$3,0,'directory')")
        .bind(other.id.as_str()).bind(boundary.id.as_str()).bind(&tree_id).execute(&db).await.is_err());
    let after: i64 = sqlx::query_scalar("SELECT mutation_serial FROM roots WHERE id=$1")
        .bind(r.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(after, before + 1);
    let pin = w.pin_version(&a, &r.id, &version).await.unwrap();
    sqlx::query("UPDATE file_versions SET retired=true WHERE id='snapshot'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        w.snapshot_entry(&a, &r.id, &version, &["empty".parse().unwrap()])
            .await
            .unwrap_err()
            .0,
        ErrorCode::NotFound
    );
    w.renew_read_pin(&a, &r.id, &pin).await.unwrap();
    w.release_read_pin(&a, &pin).await.unwrap();
}
