use std::{net::SocketAddr, path::PathBuf};

use sct_core::catalog::Catalog;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind: SocketAddr = std::env::var("SCT_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .map_err(|_| "invalid SCT_BIND socket address")?;
    let web_directory =
        PathBuf::from(std::env::var("SCT_WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    if !web_directory.join("index.html").is_file() {
        return Err("web build missing; run npm --prefix web run build or set SCT_WEB_DIR".into());
    }
    let catalog = match std::env::var("SCT_DATABASE_URL") {
        Ok(url) => Some(Catalog::new(&url).map_err(|_| "invalid SCT_DATABASE_URL")?),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("SCT_DATABASE_URL must be valid UTF-8".into()),
    };
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("ScT listening on http://{}", listener.local_addr()?);
    axum::serve(listener, sct_server::router(catalog.clone(), web_directory))
        .with_graceful_shutdown(shutdown())
        .await?;
    if let Some(catalog) = catalog {
        catalog.close().await;
    }
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("install shutdown handler");
}
