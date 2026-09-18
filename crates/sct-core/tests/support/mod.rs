use sct_core::catalog::Catalog;
use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn database() -> (Catalog, PgPool) {
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
    catalog.migrate().await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    (catalog, pool)
}
