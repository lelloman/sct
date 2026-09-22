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
    std::fs::create_dir_all(dir.join("payloads"))?;
    let mut config = json!({"bind":format!("127.0.0.1:{port}"),"public_url":format!("http://127.0.0.1:{port}"),"web_dir":web,"database_url_env":"SCT_E2E_DATABASE_URL","development":true,"enable_test_adapter":true,"pools":[{"id":"pool","capacity_bytes":10000,"overhead_bytes":0,"staging_bytes":1000}],"backends":[{"pool_id":"pool","settings":{"destination":"browser"},"inspection":{"id":"test_backend","display_name":"Browser test adapter","adapter_kind":"test","hints":{},"observed_health":"unknown"}}]});
    config["checkpoint_dir"] = json!(dir.join("checkpoints"));
    config["pools"].as_array_mut().unwrap().push(json!({"id":"browser_pool","capacity_bytes":2000000,"overhead_bytes":0,"staging_bytes":200000}));
    config["backends"].as_array_mut().unwrap().push(json!({"pool_id":"browser_pool","settings":{"directory":dir.join("payloads")},"inspection":{"id":"browser_local","display_name":"Browser payload storage","adapter_kind":"local","hints":{},"observed_health":"unknown"}}));
    // Real executable and ISO generation for browser flow qualification. Full
    // vendored production bundles are independently tested by the bundle suite.
    let bundle_dir = dir.join("archive-bundle");
    std::fs::create_dir_all(bundle_dir.join("recovery"))?;
    std::fs::copy(
        std::env::var("SCT_TEST_RECOVERY_BINARY")?,
        bundle_dir.join("recovery/sct-recover"),
    )?;
    std::fs::write(bundle_dir.join("START-HERE.txt"), b"Browser integration fixture. Use recovery/sct-recover. Not a production source distribution.\n")?;
    std::fs::write(
        bundle_dir.join("recovery/source.tar.gz"),
        b"test-only source placeholder",
    )?;
    let mut files = Vec::new();
    for path in [
        "START-HERE.txt",
        "recovery/sct-recover",
        "recovery/source.tar.gz",
    ] {
        let data = std::fs::read(bundle_dir.join(path))?;
        files.push(sct_protocol::ArchiveBundleFile {
            path: path.into(),
            length: (data.len() as u64).try_into()?,
            sha256: sct_protocol::digest(&data).parse()?,
        });
    }
    let bundle = sct_protocol::ArchiveBundle {
        format: 1,
        version: "browser-integration".into(),
        platform: "linux-x86_64-glibc236".into(),
        files,
    };
    std::fs::write(
        bundle_dir.join("recovery/bundle.json"),
        sct_protocol::canonical(&bundle)?,
    )?;
    config["archive"] = json!({"directory":dir.join("archive-cache"),"bundle_directory":bundle_dir,"quota_bytes":1073741824});
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
