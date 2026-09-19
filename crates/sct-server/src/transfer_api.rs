//! Thin HTTP translation; authorization and admission live in core.
use crate::api::{ApiError, App, Auth};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use sct_core::transfer_io::TransferEngine;
use sct_protocol::*;
fn engine(app: &App) -> Result<&TransferEngine, ApiError> {
    app.transfer
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))
}
fn id<T: std::str::FromStr>(s: &str) -> Result<T, ApiError> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn status(s: TransferStatus) -> Response {
    let code = if s.admission == AdmissionState::Pending {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    (code, Json(s)).into_response()
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/roots/{root}/uploads/{upload}/chunks", get(missing))
        .route(
            "/api/v1/roots/{root}/uploads/{upload}/chunks/{digest}",
            put(chunk),
        )
        .route("/api/v1/roots/{root}/uploads/{upload}/seal", post(seal))
        .route("/api/v1/roots/{root}/uploads", post(create))
        .route("/api/v1/roots/{root}/uploads/{upload}", get(inspect))
        .route(
            "/api/v1/roots/{root}/uploads/{upload}/pages/{digest}",
            put(page),
        )
        .route("/api/v1/roots/{root}/uploads/{upload}/admit", post(admit))
        .route("/api/v1/roots/{root}/uploads/{upload}/abort", post(abort))
}
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<Json<CreateUploadRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, ApiError> {
    let body = body.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    Ok(status(
        engine(&app)?
            .create_upload(&actor, &id(&root)?, &body)
            .await?,
    ))
}
async fn inspect(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    Ok(status(
        engine(&app)?
            .upload_status(&actor, &id(&root)?, &id(&upload)?)
            .await?,
    ))
}
async fn page(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload, digest)): Path<(String, String, String)>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let root = id(&root)?;
    let upload = id(&upload)?;
    let digest = id(&digest)?;
    // Authorize the session before consuming even a manifest page body.
    engine(&app)?.upload_status(&actor, &root, &upload).await?;
    let bytes = axum::body::to_bytes(request.into_body(), MANIFEST_BYTES)
        .await
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))?;
    engine(&app)?
        .upload_page(&actor, &root, &upload, &digest, &bytes)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn admit(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    Ok(status(
        engine(&app)?
            .admit_upload(&actor, &id(&root)?, &id(&upload)?)
            .await?,
    ))
}
async fn abort(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    Ok(status(
        engine(&app)?
            .abort_upload(&actor, &id(&root)?, &id(&upload)?)
            .await?,
    ))
}

#[derive(serde::Deserialize)]
struct MissingQuery {
    cursor: Option<CursorToken>,
}
async fn missing(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
    Query(query): Query<MissingQuery>,
) -> Result<Json<MissingChunkPage>, ApiError> {
    Ok(Json(
        engine(&app)?
            .missing_chunks(&actor, &id(&root)?, &id(&upload)?, query.cursor.as_ref())
            .await?,
    ))
}
async fn chunk(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload, digest)): Path<(String, String, String)>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let root = id(&root)?;
    let upload = id(&upload)?;
    let digest = id(&digest)?;
    let engine = engine(&app)?;
    let length = engine
        .chunk_permission(&actor, &root, &upload, &digest)
        .await?;
    let _permit = engine.payload_permit().await?;
    let bytes = axum::body::to_bytes(request.into_body(), length as usize + 1)
        .await
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))?;
    engine
        .upload_chunk(&actor, &root, &upload, &digest, bytes.to_vec())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn seal(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let status = engine(&app)?
        .seal_upload(&actor, &id(&root)?, &id(&upload)?)
        .await?;
    let code = if status.upload.state == SessionState::Ready {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((code, Json(status)).into_response())
}
