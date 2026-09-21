#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::backend::Registry;
use sct_protocol::*;
use serde_json::json;
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn browser_archives_require_csrf_keep_account_isolation_and_forbid_source_mutation() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin_token = writer.recover_administrator().await.unwrap();
    let admin = writer
        .authenticate(&admin_token.secret, false, None)
        .await
        .unwrap();
    writer
        .create_principal(&admin, &"user".parse().unwrap())
        .await
        .unwrap();
    let token = writer
        .issue_token(&admin, &"user".parse().unwrap())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry: Registry::default(),
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: None,
    };
    let server = tokio::spawn(async move {
        simple_server::axum::serve(listener, sct_server::application(app, "nonexistent".into()))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let session = http
        .post(format!("{origin}/auth/session"))
        .bearer_auth(&token.secret)
        .header("origin", &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 200);
    let cookies: Vec<_> = session
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .collect();
    let csrf = cookies
        .iter()
        .find_map(|c| c.strip_prefix("sct_csrf="))
        .unwrap();
    let cookie = cookies.join("; ");
    let body =
        json!({"request_id":"collection","label":"Optical","media_capacity_bytes":25000000000u64});
    assert_eq!(
        http.post(format!("{origin}/api/v1/archives"))
            .header("cookie", &cookie)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = http
        .post(format!("{origin}/api/v1/archives"))
        .header("cookie", &cookie)
        .header("origin", &origin)
        .header("x-csrf-token", csrf)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let collection = response.json::<ArchiveCollection>().await.unwrap();
    assert_eq!(collection.owner.as_str(), "user");
    let page = http
        .get(format!("{origin}/api/v1/archives?limit=1"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json::<ArchiveCollectionPage>()
        .await
        .unwrap();
    assert_eq!(page.collections.len(), 1);
    writer
        .create_principal(&admin, &"stranger".parse().unwrap())
        .await
        .unwrap();
    let stranger = writer
        .issue_token(&admin, &"stranger".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/archives/{}",
            collection.id.as_str()
        ))
        .bearer_auth(&stranger.secret)
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/archives/{}",
            collection.id.as_str()
        ))
        .bearer_auth(&admin_token.secret)
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    // The existing session boundary remains in force even for administrators.
    for path in [
        "/api/v1/roots/root/uploads",
        "/api/v1/roots/root/tree-drafts",
    ] {
        assert_eq!(
            http.post(format!("{origin}{path}"))
                .header("cookie", &cookie)
                .header("origin", &origin)
                .header("x-csrf-token", csrf)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        http.get(format!("{origin}/api/v1/archives?limit=1001"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    server.abort();
    writer.release().await.unwrap();
}
