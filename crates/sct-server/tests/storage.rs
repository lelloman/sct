#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::backend::{Registry, TestAdapter};
use sct_protocol::*;
use serde_json::json;
use std::sync::Arc;
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn storage_http_requires_admin_and_persists_revision_checked_configuration() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    writer
        .create_principal(&actor, &"user".parse().unwrap())
        .await
        .unwrap();
    let user = writer
        .issue_token(&actor, &"user".parse().unwrap())
        .await
        .unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    writer.configure(&registry, &[], &[]).await.unwrap();
    let listener = sct_server::web::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: None,
    };
    let server = tokio::spawn(async move {
        sct_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::lifecycle::Shutdown::new(),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::new();
    let request = json!({"id":"managed","capacity_bytes":1000,"staging_bytes":100});
    assert_eq!(
        http.post(format!("{origin}/api/v1/admin/pools"))
            .bearer_auth(&user.secret)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let pool = http
        .post(format!("{origin}/api/v1/admin/pools"))
        .bearer_auth(&admin.secret)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(pool.status(), 200);
    let pool: PoolInspection = pool.json().await.unwrap();
    let resize = json!({"expected_revision":pool.revision,"capacity_bytes":2000});
    assert_eq!(
        http.patch(format!("{origin}/api/v1/admin/pools/managed"))
            .bearer_auth(&admin.secret)
            .json(&resize)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.patch(format!("{origin}/api/v1/admin/pools/managed"))
            .bearer_auth(&admin.secret)
            .json(&resize)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    let backend = json!({"expected_revision":null,"pool_id":"managed","inspection":{"id":"one","display_name":"One","adapter_kind":"test","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null},"settings":{"destination":"one"}});
    let configured = http
        .put(format!("{origin}/api/v1/admin/backends/one"))
        .bearer_auth(&admin.secret)
        .json(&backend)
        .send()
        .await
        .unwrap();
    assert_eq!(configured.status(), 200);
    let configured: BackendConfiguration = configured.json().await.unwrap();
    assert_eq!(
        configured.inspection.observed_health,
        ObservedHealth::Reachable
    );
    assert_eq!(
        http.put(format!("{origin}/api/v1/admin/backends/mismatch"))
            .bearer_auth(&admin.secret)
            .json(&backend)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    for path in [
        "pools?limit=1",
        "backends?limit=1",
        "backends/one/configuration",
    ] {
        assert_eq!(
            http.get(format!("{origin}/api/v1/admin/{path}"))
                .bearer_auth(&user.secret)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let list = http
        .get(format!("{origin}/api/v1/admin/backends?limit=1"))
        .bearer_auth(&admin.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    assert_eq!(
        list.json::<BackendPage>().await.unwrap().backends,
        vec![configured]
    );
    server.abort();
}
