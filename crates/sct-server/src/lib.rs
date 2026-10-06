//! HTTP translation and runtime composition; domain decisions belong in sct-core.

use std::path::PathBuf;

use crate::web::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{any, get},
};
use sct_core::catalog::Catalog;
use sct_protocol::{ErrorCode, ErrorResponse};

pub fn router(catalog: Option<Catalog>, web_directory: PathBuf) -> Router {
    Router::new()
        .route(
            "/health/live",
            crate::web::routing::get_service(simple_server::health::Probe::liveness().endpoint(
                |result| {
                    result.expect("infallible liveness");
                    crate::web::http::Response::builder()
                        .header("content-type", "text/plain; charset=utf-8")
                        .body(crate::web::body::Body::from("ok\n"))
                        .expect("static liveness response")
                },
            )),
        )
        .route("/health/database", get(database_health))
        // Reserve API paths so missing API operations never return web HTML.
        .route("/api", any(api_not_implemented))
        .route("/api/{*path}", any(api_not_implemented))
        .fallback_static_dir(web_directory)
        .with_state(catalog)
}

async fn api_not_implemented() -> impl IntoResponse {
    // Generate server correlation IDs; never echo unvalidated request headers.
    let id = correlation::request_id();
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
    let ready = match catalog {
        Some(catalog) => simple_server::health::Probe::readiness(
            simple_server::health::Check::new("database", move || {
                let catalog = catalog.clone();
                async move {
                    simple_server::time::timeout(std::time::Duration::from_secs(2), catalog.ping())
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .ok_or(())
                }
            }),
        )
        .run()
        .await
        .is_ok(),
        None => false,
    };
    if ready {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

pub mod api;
mod archive_api;
pub mod config;
pub mod oidc;

pub fn application(app: api::App, web_directory: PathBuf) -> Router {
    let router =
        router(Some(app.writer.catalog().clone()), web_directory).merge(api::routes(app.clone()));
    observability::install(router, app)
        .layer(crate::web::middleware::from_fn(correlation::middleware))
}
pub fn capabilities() -> sct_protocol::Capabilities {
    use sct_protocol::*;
    Capabilities {
        protocol_version: 1,
        supported_operations: FOUNDATION_OPERATIONS
            .iter()
            .chain(TRANSFER_OPERATIONS)
            .chain(TREE_OPERATIONS)
            .chain(MANAGEMENT_OPERATIONS)
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

mod management_api;

mod storage_api;

mod inspection_api;

mod checkpoint_api;
mod maintenance_api;

mod observability;

mod correlation;

pub mod web;
