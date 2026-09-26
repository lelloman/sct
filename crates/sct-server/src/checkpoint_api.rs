use crate::api::{ApiError, App, Auth};
use sct_protocol::*;
use simple_server::extract::RejectionResponse;
use simple_server::web::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tokio::io::AsyncReadExt;
type Result<T> = std::result::Result<T, ApiError>;
fn id(s: &str) -> Result<OperationId> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn body<T>(b: std::result::Result<Json<T>, RejectionResponse>) -> Result<T> {
    Ok(b.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
fn list(q: std::result::Result<Query<ListRequest>, RejectionResponse>) -> Result<ListRequest> {
    Ok(q.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/admin/checkpoints", get(all).post(create))
        .route("/api/v1/admin/checkpoints/{checkpoint}", get(inspect))
        .route(
            "/api/v1/admin/checkpoints/{checkpoint}/abandon",
            post(release),
        )
        .route(
            "/api/v1/admin/checkpoints/{checkpoint}/inventory",
            get(inventory),
        )
        .route(
            "/api/v1/admin/checkpoints/{checkpoint}/descriptor",
            get(descriptor),
        )
        .route(
            "/api/v1/admin/checkpoints/{checkpoint}/artifacts/{artifact}",
            get(artifact),
        )
}
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    b: std::result::Result<Json<CreateCheckpointRequest>, RejectionResponse>,
) -> Result<Response> {
    let engine = app
        .transfer
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))?;
    if engine.checkpoint_directory().is_none() {
        return Err(ApiError(ErrorCode::UnsupportedOperation));
    }
    let checkpoint = app.writer.create_checkpoint(&actor, &body(b)?).await?;
    Ok((StatusCode::ACCEPTED, Json(checkpoint)).into_response())
}
async fn all(
    State(app): State<App>,
    Auth(actor): Auth,
    q: std::result::Result<Query<ListRequest>, RejectionResponse>,
) -> Result<Json<CheckpointPage>> {
    Ok(Json(app.writer.list_checkpoints(&actor, &list(q)?).await?))
}
async fn inspect(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(checkpoint): Path<String>,
) -> Result<Json<CheckpointInspection>> {
    Ok(Json(
        app.writer.checkpoint(&actor, &id(&checkpoint)?).await?,
    ))
}
async fn release(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(checkpoint): Path<String>,
    b: std::result::Result<Json<ReleaseCheckpointRequest>, RejectionResponse>,
) -> Result<Json<CheckpointInspection>> {
    Ok(Json(
        app.writer
            .release_checkpoint(&actor, &id(&checkpoint)?, &body(b)?)
            .await?,
    ))
}
async fn inventory(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(checkpoint): Path<String>,
    q: std::result::Result<Query<ListRequest>, RejectionResponse>,
) -> Result<Json<CheckpointInventoryPage>> {
    Ok(Json(
        app.writer
            .checkpoint_inventory(&actor, &id(&checkpoint)?, &list(q)?)
            .await?,
    ))
}
async fn descriptor(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(checkpoint): Path<String>,
) -> Result<Json<CheckpointDescriptor>> {
    Ok(Json(
        app.writer
            .checkpoint_descriptor(&actor, &id(&checkpoint)?)
            .await?,
    ))
}
async fn artifact(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((checkpoint, artifact)): Path<(String, String)>,
) -> Result<Response> {
    let engine = app
        .transfer
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))?;
    let file = engine
        .checkpoint_artifact(&actor, &id(&checkpoint)?, &artifact)
        .await?;
    let stream = futures_util::stream::try_unfold(file, |mut file| async move {
        let mut bytes = vec![0u8; 65536];
        let n = file.read(&mut bytes).await?;
        if n == 0 {
            Ok::<_, std::io::Error>(None)
        } else {
            bytes.truncate(n);
            Ok(Some((bytes, file)))
        }
    });
    let mut response = Body::from_stream(stream).into_response();
    response
        .headers_mut()
        .insert("content-type", "application/octet-stream".parse().unwrap());
    response.headers_mut().insert(
        "content-disposition",
        format!("attachment; filename=\"{artifact}\"")
            .parse()
            .map_err(|_| ApiError(ErrorCode::InvalidRequest))?,
    );
    Ok(response)
}
