//! Thin HTTP translation; authorization and admission live in core.
use crate::api::{ApiError, App, Auth};
use sct_core::transfer_io::TransferEngine;
use sct_protocol::*;
use simple_server::web::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
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
    pending_response(code, s)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route(
            "/api/v1/roots/{root}/versions/{version}/manifest",
            get(manifest),
        )
        .route(
            "/api/v1/roots/{root}/versions/{version}/content",
            get(content),
        )
        .route("/api/v1/roots/{root}/uploads/{upload}/commit", post(commit))
        .route(
            "/api/v1/roots/{root}/entries/{entry}/versions",
            get(history),
        )
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
fn pending_response(code: StatusCode, status: TransferStatus) -> Response {
    let mut response = (code, Json(status)).into_response();
    if code == StatusCode::ACCEPTED {
        response
            .headers_mut()
            .insert("retry-after", "1".parse().unwrap());
    }
    response
}
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<Json<CreateUploadRequest>, simple_server::extract::RejectionResponse>,
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
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        simple_server::web::body::to_bytes(request.into_body(), MANIFEST_BYTES),
    )
    .await
    .map_err(|_| ApiError(ErrorCode::DestinationUnavailable))?
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
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        simple_server::web::body::to_bytes(request.into_body(), length as usize + 1),
    )
    .await
    .map_err(|_| ApiError(ErrorCode::DestinationUnavailable))?
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
    Ok(pending_response(code, status))
}

async fn commit(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, upload)): Path<(String, String)>,
) -> Result<Json<CommitReceipt>, ApiError> {
    Ok(Json(
        engine(&app)?
            .commit_upload(&actor, &id(&root)?, &id(&upload)?)
            .await?,
    ))
}
async fn history(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, entry)): Path<(String, String)>,
    Query(query): Query<MissingQuery>,
) -> Result<Json<VersionPage>, ApiError> {
    Ok(Json(
        engine(&app)?
            .file_history(&actor, &id(&root)?, &id(&entry)?, query.cursor.as_ref())
            .await?,
    ))
}

async fn manifest(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let manifest = engine(&app)?
        .file_manifest(&actor, &id(&root)?, &id(&version)?)
        .await?;
    Ok((
        [("content-type", "application/json")],
        canonical(&manifest).map_err(|_| ApiError(ErrorCode::InternalError))?,
    )
        .into_response())
}
async fn content(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
    headers: simple_server::web::http::HeaderMap,
) -> Result<Response, ApiError> {
    content_response(
        engine(&app)?,
        &actor,
        id(&root)?,
        id(&version)?,
        None,
        headers,
    )
    .await
}

pub(crate) async fn content_response(
    engine: &TransferEngine,
    actor: &sct_core::auth::Actor,
    root: RootId,
    version: VersionId,
    path: Option<RootPath>,
    headers: simple_server::web::http::HeaderMap,
) -> Result<Response, ApiError> {
    let manifest = if let Some(path) = &path {
        engine
            .snapshot_file_manifest(actor, &root, &version, path)
            .await?
    } else {
        engine.file_manifest(actor, &root, &version).await?
    };
    let length = manifest.length.get();
    let range = if let Some(header) = headers.get("range") {
        let range = header
            .to_str()
            .ok()
            .and_then(|s| s.strip_prefix("bytes="))
            .ok_or(ApiError(ErrorCode::InvalidRequest))?;
        let (a, b) = range
            .split_once('-')
            .ok_or(ApiError(ErrorCode::InvalidRequest))?;
        let (start, end) = if a.is_empty() {
            let suffix: u64 = b.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))?;
            (length.saturating_sub(suffix), length)
        } else {
            let start: u64 = a.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))?;
            let end = if b.is_empty() {
                length
            } else {
                b.parse::<u64>()
                    .map_err(|_| ApiError(ErrorCode::InvalidRequest))?
                    .saturating_add(1)
                    .min(length)
            };
            (start, end)
        };
        if start >= end || end > length {
            let mut response = ApiError(ErrorCode::InvalidRequest).into_response();
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            response.headers_mut().insert(
                "content-range",
                format!("bytes */{length}").parse().unwrap(),
            );
            return Ok(response);
        }
        Some((start, end))
    } else {
        None
    };
    let download = if let Some(path) = &path {
        engine
            .download_snapshot_file(actor, &root, &version, path, range)
            .await?
    } else {
        engine.download(actor, &root, &version, range).await?
    };
    let start = download.start;
    let end = download.end;
    let stream = futures_util::stream::try_unfold(download, |mut d| async move {
        d.next()
            .await
            .map(|next| next.map(|bytes| (bytes, d)))
            .map_err(std::io::Error::other)
    });
    let mut response = simple_server::web::body::Body::from_stream(stream).into_response();
    response
        .headers_mut()
        .insert("content-type", "application/octet-stream".parse().unwrap());
    response
        .headers_mut()
        .insert("content-length", (end - start).to_string().parse().unwrap());
    response
        .headers_mut()
        .insert("accept-ranges", "bytes".parse().unwrap());
    if range.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            "content-range",
            format!("bytes {start}-{}/{length}", end - 1)
                .parse()
                .unwrap(),
        );
    }
    Ok(response)
}
