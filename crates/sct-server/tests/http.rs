use std::path::PathBuf;

#[tokio::test]
async fn sdk_reaches_live_server_without_advertising_unimplemented_storage() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(
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
    for path in ["/api/v1/roots", "/api/v1/uploads"] {
        assert_eq!(
            http.get(format!("{origin}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
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
