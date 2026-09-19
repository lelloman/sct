//! HTTP translation and runtime composition; domain decisions belong in sct-core.

use std::path::PathBuf;

use sct_core::catalog::Catalog;
use sct_protocol::{ErrorCode, ErrorResponse};
use simple_server::axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{any, get},
};
use tower_http::services::ServeDir;

pub fn router(catalog: Option<Catalog>, web_directory: PathBuf) -> Router {
    Router::new()
        .route("/health/live", get(|| async { "ok\n" }))
        .route("/health/database", get(database_health))
        // Reserve API paths so missing API operations never return web HTML.
        .route("/api", any(api_not_implemented))
        .route("/api/{*path}", any(api_not_implemented))
        .fallback_service(ServeDir::new(web_directory))
        .with_state(catalog)
}

async fn api_not_implemented() -> impl IntoResponse {
    // Generate server correlation IDs; never echo unvalidated request headers.
    let id = uuid::Uuid::new_v4().simple().to_string();
    let body = ErrorResponse::new(
        id.parse().expect("UUID is a valid request ID"),
        ErrorCode::UnsupportedOperation,
    );
    (
        StatusCode::NOT_FOUND,
        [("x-request-id", id), ("cache-control", "no-store".into())],
        Json(body),
    )
}

async fn database_health(State(catalog): State<Option<Catalog>>) -> StatusCode {
    match catalog {
        Some(catalog) if catalog.ping().await.is_ok() => StatusCode::NO_CONTENT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

pub mod api;
pub mod config;
pub mod oidc;

pub fn application(app: api::App, web_directory: PathBuf) -> Router {
    router(Some(app.writer.catalog().clone()), web_directory).merge(api::routes(app))
}
pub fn capabilities() -> sct_protocol::Capabilities {
    use sct_protocol::*;
    Capabilities {
        protocol_version: 1,
        supported_operations: FOUNDATION_OPERATIONS
            .iter()
            .chain(TRANSFER_OPERATIONS)
            .chain(TREE_OPERATIONS)
            .map(|s| (*s).into())
            .collect(),
        storage_format_versions: vec![1],
        chunk_profiles: vec![CHUNK_PROFILE.into()],
        limits: RequestLimits {
            max_page_size: 1000.try_into().unwrap(),
            max_request_bytes: 65536.try_into().unwrap(),
            max_metadata_bytes: (MANIFEST_BYTES as u64).try_into().unwrap(),
            max_manifest_page_bytes: (MANIFEST_BYTES as u64).try_into().unwrap(),
            max_manifest_page_entries: (MANIFEST_FANOUT as u64).try_into().unwrap(),
            max_chunk_batch: 1000.try_into().unwrap(),
        },
    }
}

mod lifecycle_api;
mod transfer_api;
mod tree_api;
