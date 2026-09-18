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

