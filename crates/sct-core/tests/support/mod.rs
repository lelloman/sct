use sct_core::catalog::Catalog;
use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn database() -> (Catalog, PgPool) {
    database_through(None).await
}

pub async fn database_through(version: Option<i64>) -> (Catalog, PgPool) {
    let url = std::env::var("SCT_TEST_DATABASE_URL").expect("dedicated test database required");
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let separator = if url.contains('?') { '&' } else { '?' };
    let url = format!("{url}{separator}options=-csearch_path%3D{schema}");
    let catalog = Catalog::new(&url).unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    if let Some(version) = version {
        let mut migrator = sqlx::migrate::Migrator::new(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations"
        )))
        .await
        .unwrap();
        migrator.migrations = migrator
            .migrations
            .into_owned()
            .into_iter()
            .filter(|m| m.version <= version)
            .collect::<Vec<_>>()
            .into();
        migrator.run(&pool).await.unwrap();
    } else {
        catalog.migrate().await.unwrap();
    }
    (catalog, pool)
}
