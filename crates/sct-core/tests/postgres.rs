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
