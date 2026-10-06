//! Exercise the production fallback, including its separation from API routes.
use simple_server::lifecycle::Shutdown;
use std::{path::PathBuf, time::Duration};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("sct-static-contract-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "<html>ScT</html>").unwrap();
        std::fs::write(root.join("assets/index.html"), "asset index").unwrap();
        std::fs::write(root.join("assets/app.js"), "0123456789").unwrap();
        Self(root)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn production_static_fallback_preserves_http_and_api_boundaries() {
    let files = Files::new();
    let listener = sct_server::web::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let shutdown = Shutdown::new();
    let server = tokio::spawn(sct_server::web::serve(
        listener,
        sct_server::router(None, files.0.clone()),
        shutdown.clone(),
    ));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client.get(format!("{origin}/")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(!response.headers().contains_key("cache-control"));
    assert!(!response.headers().contains_key("x-request-id"));
    assert_eq!(response.text().await.unwrap(), "<html>ScT</html>");
    let response = client
        .head(format!("{origin}/assets/app.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-length"], "10");
    let modified = response.headers()["last-modified"].clone();
    assert!(response.bytes().await.unwrap().is_empty());
    let response = client
        .get(format!("{origin}/assets/app.js"))
        .header("range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 2-5/10");
    assert_eq!(response.text().await.unwrap(), "2345");
    assert_eq!(
        client
            .get(format!("{origin}/assets/app.js"))
            .header("if-modified-since", modified)
            .send()
            .await
            .unwrap()
            .status(),
        304
    );
    let redirect = client
        .get(format!("{origin}/assets?version=1"))
        .send()
        .await
        .unwrap();
    assert!(redirect.status().is_redirection());
    assert_eq!(redirect.headers()["location"], "/assets/?version=1");
    assert_eq!(
        client
            .get(format!("{origin}/assets/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "asset index"
    );
    // The fallback is a file server, not an implicit SPA catch-all.
    for path in [
        "/missing",
        "/assets/missing.js",
        "/%2e%2e/outside.txt",
        "/assets/%2e%2e/%2e%2e/outside.txt",
    ] {
        assert_eq!(
            client
                .get(format!("{origin}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404,
            "{path}"
        );
    }
    assert_eq!(
        client
            .post(format!("{origin}/assets/app.js"))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    std::fs::write(files.0.join("api"), "must never serve API content").unwrap();
    for (method, path) in [
        (reqwest::Method::GET, "/api"),
        (reqwest::Method::GET, "/api/unknown"),
        (reqwest::Method::POST, "/api/unknown"),
    ] {
        let response = client
            .request(method, format!("{origin}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().contains_key("x-request-id"));
        let error: sct_protocol::ErrorResponse = response.json().await.unwrap();
        assert_eq!(error.code, sct_protocol::ErrorCode::UnsupportedOperation);
    }
    shutdown.request();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
