use simple_server::database::postgres as db;
mod support;

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL; run scripts/test-integration"]
async fn lease_excludes_second_writer_and_fences_expired_owner() {
    let (catalog, pool) = support::database().await;
    let first = catalog.acquire_writer().await.unwrap();
    assert!(first.ready().await);
    assert!(catalog.acquire_writer().await.is_err());
    db::query("UPDATE server_lease SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!first.ready().await);
    assert!(first.renew().await.is_err());
    let second = catalog.acquire_writer().await.unwrap();
    assert!(!first.ready().await);
    first.release().await.unwrap();
    assert!(second.ready().await);
    second.release().await.unwrap();
    assert!(catalog.acquire_writer().await.is_ok());
}
