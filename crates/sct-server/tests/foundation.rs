#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use openidconnect::{
    AccessToken, Audience, EmptyAdditionalClaims, IssuerUrl, JsonWebKeyId, Nonce,
    PrivateSigningKey, StandardClaims, SubjectIdentifier,
    core::{CoreIdToken, CoreIdTokenClaims, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey},
};
use sct_core::{
    backend::{Registry, TestAdapter},
    roots::{BackendSetup, PoolSetup},
};
use sct_server::{
    api::App,
    config::{Config, OidcConfig},
    oidc::Oidc,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use simple_server::axum::{
    Json, Router,
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
struct Provider {
    origin: String,
    key: Arc<CoreRsaPrivateSigningKey>,
    codes: Arc<Mutex<HashMap<String, HashMap<String, String>>>>,
}
async fn discovery(State(p): State<Provider>) -> Json<Value> {
    Json(
        json!({"issuer":p.origin,"authorization_endpoint":format!("{}/authorize",p.origin),"token_endpoint":format!("{}/token",p.origin),"jwks_uri":format!("{}/jwks",p.origin),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["client_secret_basic"],"code_challenge_methods_supported":["S256"]}),
    )
}
async fn jwks(State(p): State<Provider>) -> Json<Value> {
    Json(json!({"keys":[p.key.as_verification_key()]}))
}
async fn authorize(
    State(p): State<Provider>,
    Query(q): Query<HashMap<String, String>>,
) -> Redirect {
    assert_eq!(q.get("code_challenge_method").unwrap(), "S256");
    let code = uuid::Uuid::new_v4().simple().to_string();
    let mut redirect = reqwest::Url::parse(&q["redirect_uri"]).unwrap();
    redirect
        .query_pairs_mut()
        .append_pair("state", &q["state"])
        .append_pair("code", &code);
    p.codes.lock().unwrap().insert(code, q);
    Redirect::to(redirect.as_str())
}
async fn token(State(p): State<Provider>, Form(form): Form<HashMap<String, String>>) -> Response {
    let Some(q) = p.codes.lock().unwrap().remove(&form["code"]) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes()));
    if challenge != q["code_challenge"] {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mode = q.get("test_mode").map(String::as_str).unwrap_or("ok");
    let issuer = if mode == "issuer" {
        "https://wrong.example"
    } else {
        &p.origin
    };
    let audience = if mode == "audience" {
        "wrong-client"
    } else {
        "sct-test"
    };
    let expiry = if mode == "expired" { -600 } else { 300 };
    let nonce = if mode == "nonce" {
        "wrong-nonce"
    } else {
        &q["nonce"]
    };
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(issuer.into()).unwrap(),
        vec![Audience::new(audience.into())],
        chrono::Utc::now() + chrono::Duration::seconds(expiry),
        chrono::Utc::now(),
        StandardClaims::new(SubjectIdentifier::new("subject-a".into())),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(nonce.into())));
    let access = AccessToken::new("test-access-token".into());
    let token = CoreIdToken::new(
        claims,
        p.key.as_ref(),
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        Some(&access),
        None,
    )
    .unwrap();
    let mut signed = token.to_string();
    if mode == "signature" {
        // Flip the first encoded signature character, not trailing padding bits.
        let start = signed.rfind('.').unwrap() + 1;
        let replacement = if &signed[start..start + 1] == "a" {
            "b"
        } else {
            "a"
        };
        signed.replace_range(start..start + 1, replacement);
    }
    Json(json!({"access_token":if mode=="access_hash"{"substituted"}else{access.secret()},"token_type":"Bearer","id_token":signed})).into_response()
}
fn cookies(response: &reqwest::Response) -> String {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|h| h.to_str().unwrap().split(';').next().unwrap())
        .collect::<Vec<_>>()
        .join("; ")
}
async fn oidc_login(
    http: &reqwest::Client,
    origin: &str,
    mode: &str,
) -> (reqwest::Response, String, String) {
    let begin = http
        .get(format!("{origin}/auth/login"))
        .send()
        .await
        .unwrap();
    assert_eq!(begin.status(), 303);
    let browser = cookies(&begin);
    let mut authorize = reqwest::Url::parse(begin.headers()["location"].to_str().unwrap()).unwrap();
    authorize.query_pairs_mut().append_pair("test_mode", mode);
    let redirect = http.get(authorize).send().await.unwrap();
    let callback = redirect.headers()["location"].to_str().unwrap().to_owned();
    let response = http
        .get(&callback)
        .header("cookie", &browser)
        .send()
        .await
        .unwrap();
    (response, callback, browser)
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn oidc_http_authentication_csrf_and_foundation_api() {
    let (catalog, _pool) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let provider_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_origin = format!("http://{}", provider_listener.local_addr().unwrap());
    let provider = Provider {
        origin: provider_origin.clone(),
        key: Arc::new(
            CoreRsaPrivateSigningKey::from_pem(
                include_str!("fixtures/oidc-test-key.pem"),
                Some(JsonWebKeyId::new("test-key".into())),
            )
            .unwrap(),
        ),
        codes: Arc::new(Mutex::new(HashMap::new())),
    };
    let provider_task = tokio::spawn(async move {
        simple_server::axum::serve(
            provider_listener,
            Router::new()
                .route("/.well-known/openid-configuration", get(discovery))
                .route("/jwks", get(jwks))
                .route("/authorize", get(authorize))
                .route("/token", post(token))
                .with_state(provider),
        )
        .await
        .unwrap()
    });
    // The test database URL is merely a readily available test-only client secret;
    // the mock provider does not inspect it, and no production credential is used.
    let oidc_config = OidcConfig {
        issuer: provider_origin,
        client_id: "sct-test".into(),
        client_secret_env: "SCT_TEST_DATABASE_URL".into(),
    };
    let config=Config::parse(&format!("bind='127.0.0.1:0'\npublic_url='{origin}'\nweb_dir='web/dist'\ndatabase_url_env='SCT_TEST_DATABASE_URL'\ndevelopment=true\n")).unwrap();
    let oidc = Oidc::discover(&config, &oidc_config).await.unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    let backend=BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":"one","display_name":"Test","adapter_kind":"test","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings:json!({"destination":"one"})};
    writer
        .configure(
            &registry,
            &[PoolSetup {
                id: "pool".parse().unwrap(),
                capacity: 10000.try_into().unwrap(),
                staging: 1000.try_into().unwrap(),
            }],
            &[backend],
        )
        .await
        .unwrap();
    let app = App {
        transfer: None,
        writer: writer.clone(),
        registry,
        oidc: Some(Arc::new(oidc)),
        public_url: origin.clone(),
        secure_cookies: false,
    };
    let server = tokio::spawn(async move {
        simple_server::axum::serve(
            listener,
            sct_server::application(app, PathBuf::from("nonexistent")),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    assert_eq!(
        http.get(format!("{origin}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let anonymous = http
        .get(format!("{origin}/api/v1/roots?limit=10"))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    assert_eq!(anonymous.headers()["cache-control"], "no-store");
    for mode in [
        "issuer",
        "audience",
        "expired",
        "nonce",
        "signature",
        "access_hash",
    ] {
        let (response, _, _) = oidc_login(&http, &origin, mode).await;
        assert_eq!(response.status(), 401, "{mode}");
    }
    let (logged_in, callback, browser) = oidc_login(&http, &origin, "ok").await;
    assert_eq!(logged_in.status(), 303);
    let session = cookies(&logged_in);
    assert!(
        logged_in
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap().contains("HttpOnly"))
    );
    assert_eq!(
        http.get(callback)
            .header("cookie", browser)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let who: Value = http
        .get(format!("{origin}/auth/session"))
        .header("cookie", &session)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let principal = who["principal_id"].as_str().unwrap();
    let csrf = session
        .split("; ")
        .find_map(|c| c.strip_prefix("sct_csrf="))
        .unwrap();
    assert_eq!(
        http.post(format!("{origin}/auth/tokens"))
            .header("cookie", &session)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(format!("{origin}/auth/tokens"))
            .header("cookie", &session)
            .header("origin", "https://evil.example")
            .header("x-csrf-token", csrf)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let issued: Value = http
        .post(format!("{origin}/auth/tokens"))
        .header("cookie", &session)
        .header("origin", &origin)
        .header("x-csrf-token", csrf)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(issued["secret"].as_str().is_some());
    let root_body = json!({"idempotency_key":"http_root","owner_id":principal,"display_name":"Storage","mount_path":"/","placement":{"kind":"single","backend_id":"one","read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":5000,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":5000}]});
    let created = http
        .post(format!("{origin}/api/v1/roots"))
        .bearer_auth(&admin.secret)
        .json(&root_body)
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);
    let root: Value = created.json().await.unwrap();
    let root_id = root["id"].as_str().unwrap();
    let enabled=http.patch(format!("{origin}/api/v1/roots/{root_id}")).bearer_auth(&admin.secret).json(&json!({"expected_revision":root["revision"],"display_name":root["display_name"],"state":"enabled","placement":root["placement"],"quota_bytes":root["quota_bytes"]})).send().await.unwrap();
    assert_eq!(enabled.status(), 200);
    let sdk = sct_client::Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin.secret.clone())
        .unwrap();
    assert!(
        sdk.capabilities()
            .await
            .unwrap()
            .supported_operations
            .contains(&"create_directory".into())
    );
    assert_eq!(
        sdk.inspect_root(&root_id.parse().unwrap())
            .await
            .unwrap()
            .id
            .as_str(),
        root_id
    );

    let token = issued["secret"].as_str().unwrap();
    let roots: Value = http
        .get(format!("{origin}/api/v1/roots?limit=10"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(roots["items"][0]["id"], root_id);
    let parent: Value = http
        .get(format!("{origin}/api/v1/roots/{root_id}/resolve?path=%2F"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entry_response=http.post(format!("{origin}/api/v1/roots/{root_id}/entries")).bearer_auth(token).json(&json!({"idempotency_key":"http_mkdir","parent_id":parent["id"],"expected_parent_revision":parent["revision"],"name":"Documents","versioning":"independent_files"})).send().await.unwrap();
    assert_eq!(entry_response.status(), 200);
    let entry: Value = entry_response.json().await.unwrap();
    let entries: Value = http
        .get(format!(
            "{origin}/api/v1/roots/{root_id}/entries?directory_id={}&limit=10",
            parent["id"].as_str().unwrap()
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(entries["items"][0]["id"], entry["id"]);
    let policy = http
        .get(format!(
            "{origin}/api/v1/policies?kind=entry&root_id={root_id}&entry_id={}",
            entry["id"].as_str().unwrap()
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(policy.status(), 200);
    let invalid = http
        .post(format!("{origin}/api/v1/roots/{root_id}/entries"))
        .bearer_auth(token)
        .body("private-secret-malformed-json")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 400);
    let request_id = invalid.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let safe: Value = invalid.json().await.unwrap();
    assert_eq!(safe["request_id"], request_id);
    assert!(!safe.to_string().contains("private-secret"));

    for (method, path) in [
        (reqwest::Method::GET, "/api/v1/entries/%FF"),
        (reqwest::Method::PUT, "/api/v1/roots"),
    ] {
        let response = http
            .request(method, format!("{origin}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error());
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let envelope: sct_protocol::ErrorResponse = response.json().await.unwrap();
        assert_eq!(envelope.request_id.as_str(), id);
    }
    let capabilities: Value = http
        .get(format!("{origin}/api/v1/capabilities"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        capabilities["supported_operations"]
            .as_array()
            .unwrap()
            .contains(&json!("create_directory"))
    );
    assert!(
        capabilities["storage_format_versions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    writer
        .revoke_credential(&actor, issued["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        http.get(format!("{origin}/auth/session"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(format!("{origin}/auth/logout"))
            .header("cookie", &session)
            .header("origin", &origin)
            .header("x-csrf-token", csrf)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        http.get(format!("{origin}/auth/session"))
            .header("cookie", &session)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    writer.release().await.unwrap();
    assert_eq!(
        http.get(format!("{origin}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    server.abort();
    provider_task.abort();
}
