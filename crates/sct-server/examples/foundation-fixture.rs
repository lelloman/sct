//! Disposable browser-test fixture, never linked into the production server.
use sct_core::catalog::Catalog;
use serde_json::json;
use sqlx::PgPool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("fixture directory required")?,
    );
    let url = std::env::var("SCT_TEST_DATABASE_URL")?;
    let admin = PgPool::connect(&url).await?;
    if std::env::args().nth(2).as_deref() == Some("cleanup") {
        let data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("fixture.json"))?)?;
        let schema = data["schema"].as_str().ok_or("missing test schema")?;
        if !schema.starts_with("sct_e2e_")
            || !schema
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("invalid test schema".into());
        }
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await?;
        return Ok(());
    }
    let schema = format!("sct_e2e_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let separator = if url.contains('?') { '&' } else { '?' };
    let url = format!("{url}{separator}options=-csearch_path%3D{schema}");
    let catalog = Catalog::new(&url)?;
    catalog.migrate().await?;
    let writer = catalog.acquire_writer().await?;
    let token = writer.recover_administrator().await?;
    let actor = writer.authenticate(&token.secret, false, None).await?;
    writer.create_principal(&actor, &"alice".parse()?).await?;
    writer.create_principal(&actor, &"bob".parse()?).await?;
    let alice = writer.issue_token(&actor, &"alice".parse()?).await?;
    let bob = writer.issue_token(&actor, &"bob".parse()?).await?;
    writer.release().await?;
    catalog.close().await;
    let port = std::env::var("SCT_E2E_PORT").unwrap_or_else(|_| "18080".into());
    let web = std::env::current_dir()?.join("web/dist");
    let config = json!({"bind":format!("127.0.0.1:{port}"),"public_url":format!("http://127.0.0.1:{port}"),"web_dir":web,"database_url_env":"SCT_E2E_DATABASE_URL","development":true,"enable_test_adapter":true,"pools":[{"id":"pool","capacity_bytes":10000,"overhead_bytes":0,"staging_bytes":1000}],"backends":[{"pool_id":"pool","settings":{"destination":"browser"},"inspection":{"id":"test_backend","display_name":"Browser test adapter","adapter_kind":"test","hints":{},"observed_health":"unknown"}}]});
    let value: toml::Value = serde_json::from_value(config)?;
    std::fs::write(dir.join("config.toml"), toml::to_string(&value)?)?;
    std::fs::write(
        dir.join("fixture.json"),
        serde_json::to_string(
            &json!({"schema":schema,"database_url":url,"admin":token.secret,"alice":alice.secret,"bob":bob.secret}),
        )?,
    )?;
    Ok(())
}
