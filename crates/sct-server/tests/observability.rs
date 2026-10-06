use simple_server::database::postgres as db;
#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::backend::Registry;
use sct_protocol::ErrorResponse;

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn metrics_require_admin_bound_labels_and_readiness_tracks_fencing() {
    let (catalog, db) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    writer
        .create_principal(&actor, &"reader-private".parse().unwrap())
        .await
        .unwrap();
    let reader = writer
        .issue_token(&actor, &"reader-private".parse().unwrap())
        .await
        .unwrap();
    let listener = sct_server::web::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry: Registry::default(),
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: None,
    };
    let server = sct_spawn(async move {
        sct_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::engine_lifecycle::Shutdown::new(),
        )
        .await
        .unwrap();
    });
    let http = reqwest::Client::new();
    assert_eq!(
        http.get(format!("{origin}/metrics"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.get(format!("{origin}/metrics"))
            .bearer_auth(&reader.secret)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    for value in ["private-root-a", "private-root-b"] {
        let response = http
            .get(format!(
                "{origin}/api/v1/roots/{value}?secret=never-log-this"
            ))
            .header("x-request-id", "untrusted-id")
            .bearer_auth(&admin.secret)
            .send()
            .await
            .unwrap();
        let correlation = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body: ErrorResponse = response.json().await.unwrap();
        assert_eq!(correlation.len(), 32);
        assert_eq!(
            uuid::Uuid::parse_str(&correlation)
                .unwrap()
                .get_version_num(),
            4
        );
        assert_eq!(correlation, body.request_id.as_str());
        assert_ne!(correlation, "untrusted-id");
    }
    let response = http
        .get(format!("{origin}/metrics"))
        .bearer_auth(&admin.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let metrics = response.text().await.unwrap();
    assert!(metrics.contains("sct_http_responses_total{route=\"/api/v1/roots/{root}\",method=\"GET\",status_class=\"4xx\"} 2"));
    for secret in [
        &admin.secret,
        &reader.secret,
        "reader-private",
        "private-root-a",
        "private-root-b",
        "never-log-this",
        "untrusted-id",
    ] {
        assert!(!metrics.contains(secret));
    }
    assert_eq!(
        http.get(format!("{origin}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    db::query("UPDATE server_lease SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        http.get(format!("{origin}/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        http.get(format!("{origin}/health/live"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    server.abort();
}

fn sct_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(
        simple_server::runtime::Runtime::try_current()
            .expect("test engine")
            .scope(future),
    )
}
