use sct_core::catalog::Catalog;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn catalog_connects_to_real_postgres() {
    let database_url = std::env::var("SCT_TEST_DATABASE_URL")
        .expect("scripts/test-integration supplies SCT_TEST_DATABASE_URL");
    let catalog = Catalog::new(&database_url).expect("valid database configuration");
    catalog.ping().await.expect("PostgreSQL accepts queries");
    catalog.close().await;
    assert!(catalog.ping().await.is_err(), "closed pool must fail");
}

mod support;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn migration_preflight_rejects_changed_sqlx_checksum() {
    let (catalog, pool) = support::database().await;
    catalog.migrate().await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode('00', 'hex') WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    let error = catalog.migrate().await.unwrap_err();
    assert!(error.to_string().contains("DigestMismatch"), "{error}");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn migrations_constraints_and_rollback() {
    let (catalog, pool) = support::database().await;
    catalog.migrate().await.unwrap();
    sqlx::query("INSERT INTO principals(id) VALUES('alice'),('bob')")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO principals(id) VALUES('rolled_back')")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM principals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    sqlx::query("INSERT INTO pools VALUES('pool','r',10000)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO allocations(id,pool_id,capacity) VALUES('staging','pool',1000)")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE pools SET capacity=999")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE allocations SET occupied=1001")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO credentials(id,principal_id,secret_hash,kind) VALUES('x','unknown','hash','token')").execute(&pool).await.is_err());
    for (root, owner) in [("a", "alice"), ("b", "bob")] {
        sqlx::query("INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision) VALUES($1,$2,$1,'/','r','disabled','{}',4000,'{}',$1,'r')")
          .bind(root).bind(owner).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO entries(id,root_id,name,kind,revision) VALUES($1,$1,'root','directory','r')").bind(root).execute(&pool).await.unwrap();
    }
    assert!(sqlx::query("INSERT INTO entries VALUES('cross','a','b','bad','directory','r',false,NULL,NULL,NULL,NULL,'independent_files')").execute(&pool).await.is_err());
    sqlx::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) VALUES('child','a','a','Name','directory','r'),('child2','a','a','name','directory','r')").execute(&pool).await.unwrap();
    assert!(sqlx::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) VALUES('duplicate','a','a','Name','directory','r')").execute(&pool).await.is_err());
    sqlx::query("INSERT INTO allocations(id,pool_id,root_id,capacity) VALUES('a','pool','a',5000),('b','pool','b',4000)").execute(&pool).await.unwrap();
    assert!(
        sqlx::query("UPDATE allocations SET capacity=4001 WHERE id='b'")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE _sqlx_migrations SET checksum='\\x00'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(catalog.migrate().await.is_err());
    catalog.close().await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn management_upgrade_preserves_m3_accounting_and_generation_identity() {
    let (catalog, pool) = support::database_through(Some(12)).await;
    sqlx::raw_sql("INSERT INTO principals(id) VALUES('owner');
        INSERT INTO pools VALUES('pool','revision',10000);
        INSERT INTO backends VALUES('backend','pool','local','unique','{}');
        INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision)
        VALUES('root','owner','root','/','revision','enabled','{}',4000,'{}','policy','revision');
        INSERT INTO root_backends VALUES('root','backend');
        INSERT INTO allocations(id,pool_id,root_id,capacity,reserved,occupied,pending_cleanup)
        VALUES('allocation','pool','root',9000,100,200,50);
        INSERT INTO operations(id,root_id,principal_id,kind,state,server_epoch) VALUES('operation','root','owner','upload','active',1);
        INSERT INTO content_objects(root_id,digest,kind,length) VALUES('root',repeat('a',64),'chunk',200);
        INSERT INTO object_copies(generation,root_id,digest,backend_id,allocation_id,operation_id,bytes,state,worker_epoch,server_epoch)
        VALUES('generation','root',repeat('a',64),'backend','allocation','operation',200,'available',1,1);
        INSERT INTO grants VALUES('grant','root','owner','revision','[\"write\"]');")
        .execute(&pool).await.unwrap();
    catalog.migrate().await.unwrap();
    catalog.migrate().await.unwrap();
    let accounting: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT capacity,reserved,occupied,pending_cleanup FROM allocations WHERE id='allocation'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(accounting, (9000, 100, 200, 50));
    let root: (i64, i64, Option<i64>) = sqlx::query_as(
        "SELECT quota,quota_ceiling,expected_upload_interval_seconds FROM roots WHERE id='root'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(root, (4000, 4000, None));
    let grant: serde_json::Value =
        sqlx::query_scalar("SELECT capabilities FROM grants WHERE id='grant'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(grant, serde_json::json!(["write"]));
    let sequence: i64 = sqlx::query_scalar(
        "SELECT created_sequence FROM object_copies WHERE generation='generation'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(sequence > 0);
    assert!(
        sqlx::query("UPDATE object_copies SET created_sequence=created_sequence+1")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE object_copies SET state='deleting'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT created_sequence FROM object_copies WHERE generation='generation'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        sequence
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM checkpoint_barrier")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query("UPDATE allocations SET capacity=299")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE pools SET capacity=8999")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE roots SET expected_upload_interval_seconds=59")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE roots SET quota_ceiling=-1")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision) VALUES('new','owner','new','/new','revision','disabled','{}',300,'{}','new-policy','revision')")
        .execute(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT quota_ceiling FROM roots WHERE id='new'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        300
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn management_catalog_rejects_invalid_states_and_rolls_back_audit() {
    let (_, pool) = support::database().await;
    sqlx::raw_sql("INSERT INTO principals(id) VALUES('p');
        INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision)
        VALUES('r','p','root','/','rev','disabled','{}',1000,'{}','policy','rev');
        INSERT INTO maintenance_jobs(id,root_id,principal_id,run_principal_id,key,request,kind,revision)
        VALUES('j','r','p','p','key','{}','integrity_scan','rev');
        INSERT INTO checkpoints(id,principal_id,key,revision) VALUES('c','p','checkpoint-key','rev');")
        .execute(&pool).await.unwrap();
    for statement in [
        "UPDATE maintenance_jobs SET state='unknown'",
        "UPDATE maintenance_jobs SET attempts=-1",
        "UPDATE maintenance_jobs SET processed=-1",
        "UPDATE checkpoints SET state='complete'",
        "UPDATE checkpoints SET generation_watermark=-1",
        "INSERT INTO checkpoint_objects VALUES('c','missing-generation')",
        "INSERT INTO maintenance_jobs(id,root_id,principal_id,run_principal_id,key,request,kind,revision) VALUES('j2','r','p','p','key','{}','mirror_repair','rev')",
        "INSERT INTO integrity_findings(id,root_id,job_id,digest,kind) VALUES('f','r','missing-job',repeat('a',64),'missing')",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement}"
        );
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO management_audit(root_id,principal_id,event,details) VALUES('r','p','root.updated','{}')").execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM management_audit")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
