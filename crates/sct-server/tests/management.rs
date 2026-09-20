#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::backend::Registry;
use sct_protocol::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn account_http_scope_secrets_revocation_and_csrf() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
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
    let account = http
        .get(format!("{origin}/api/v1/account"))
        .bearer_auth(&admin.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(account.headers()["cache-control"], "no-store");
    assert!(
        account
            .json::<AccountInspection>()
            .await
            .unwrap()
            .administrator
    );
    let created = http
        .post(format!("{origin}/api/v1/principals"))
        .bearer_auth(&admin.secret)
        .json(&json!({"id":"user"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let user = created.json::<PrincipalInspection>().await.unwrap();
    assert!(!user.administrator);
    let issued = http
        .post(format!("{origin}/api/v1/principals/user/credentials"))
        .bearer_auth(&admin.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(issued.status(), 200);
    assert_eq!(issued.headers()["cache-control"], "no-store");
    let token = issued.json::<IssuedToken>().await.unwrap();
    let credentials = http
        .get(format!(
            "{origin}/api/v1/principals/user/credentials?limit=1"
        ))
        .bearer_auth(&token.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(credentials.status(), 200);
    let raw = credentials.text().await.unwrap();
    assert!(!raw.contains(&token.secret));
    assert!(!raw.contains("secret"));
    let page: CredentialPage = serde_json::from_str(&raw).unwrap();
    assert_eq!(page.credentials[0].id, token.id);
    for path in [
        "/api/v1/principals?limit=1",
        "/api/v1/principals/local_admin/credentials?limit=1",
    ] {
        assert_eq!(
            http.get(format!("{origin}{path}"))
                .bearer_auth(&token.secret)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/principals/user/credentials?limit=1001"
        ))
        .bearer_auth(&token.secret)
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    assert_eq!(
        http.patch(format!("{origin}/api/v1/principals/user"))
            .bearer_auth(&token.secret)
            .json(&json!({"expected_revision":user.revision,"administrator":true,"disabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.delete(format!("{origin}/api/v1/credentials/{}", token.id.as_str()))
            .bearer_auth(&token.secret)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        http.get(format!("{origin}/api/v1/account"))
            .bearer_auth(&token.secret)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    let session = writer
        .oidc_session("https://test.invalid", "subject")
        .await
        .unwrap();
    let actor = writer
        .authenticate(&session.secret, true, None)
        .await
        .unwrap();
    let url = format!(
        "{origin}/api/v1/principals/{}/credentials",
        actor.principal_id()
    );
    let cookie = format!("sct_session={}", session.secret);
    assert_eq!(
        http.post(&url)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let valid = http
        .post(&url)
        .header("cookie", &cookie)
        .header("origin", &origin)
        .header("x-csrf-token", session.csrf.as_ref().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), 200);
    server.abort();
}
