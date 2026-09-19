use std::path::PathBuf;

#[tokio::test]
async fn sdk_reaches_live_server_without_advertising_unimplemented_storage() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        simple_server::axum::serve(
            listener,
            sct_server::router(None, PathBuf::from("nonexistent-web")),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    });
    sct_client::Client::new(origin.parse().unwrap())
        .unwrap()
        .check_liveness()
        .await
        .unwrap();
    let http = reqwest::Client::new();
    let mut request_ids = std::collections::BTreeSet::new();
    for (method, path) in [
        (reqwest::Method::GET, "/api/v1/roots"),
        (reqwest::Method::POST, "/api/v1/uploads"),
    ] {
        let response = http
            .request(method, format!("{origin}{path}"))
            .header("x-request-id", "untrusted/path?secret=value")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let error: sct_protocol::ErrorResponse = response.json().await.unwrap();
        assert_eq!(error.request_id.as_str(), request_id);
        assert!(request_ids.insert(request_id));
        assert_eq!(error.code, sct_protocol::ErrorCode::UnsupportedOperation);
        assert_eq!(error.retry, sct_protocol::RetryGuidance::Never);
        assert!(!error.message.contains("secret"));
    }
    assert_eq!(
        http.get(format!("{origin}/health/database"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    stop.send(()).unwrap();
    server.await.unwrap();
}
