use crate::api::{ApiError, App, Auth};
use sct_protocol::*;
use simple_server::axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
fn id<T: std::str::FromStr>(s: &str) -> Result<T, ApiError> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/roots/{root}/policy-jobs", post(apply))
        .route("/api/v1/roots/{root}/jobs/{job}", get(job))
        .route("/api/v1/roots/{root}/jobs/{job}/advance", post(advance_job))
        .route("/api/v1/roots/{root}/jobs/{job}/resume", post(resume_job))
        .route("/api/v1/roots/{root}/policy-previews", post(create))
        .route(
            "/api/v1/roots/{root}/policy-previews/{preview}",
            get(inspect),
        )
        .route(
            "/api/v1/roots/{root}/policy-previews/{preview}/advance",
            post(advance),
        )
        .route(
            "/api/v1/roots/{root}/policy-previews/{preview}/versions",
            get(versions),
        )
}
fn engine(app: &App) -> Result<&sct_core::transfer_io::TransferEngine, ApiError> {
    app.transfer
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))
}
async fn apply(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<Json<ApplyChangeRequest>, simple_server::axum::extract::rejection::JsonRejection>,
) -> Result<Json<LifecycleJob>, ApiError> {
    Ok(Json(
        engine(&app)?
            .apply_change(
                &actor,
                &id(&root)?,
                &body.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn job(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, job)): Path<(String, String)>,
) -> Result<Json<LifecycleJob>, ApiError> {
    Ok(Json(
        engine(&app)?
            .lifecycle_job(&actor, &id(&root)?, &id(&job)?)
            .await?,
    ))
}
async fn advance_job(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, job)): Path<(String, String)>,
) -> Result<Json<LifecycleJob>, ApiError> {
    Ok(Json(
        engine(&app)?
            .advance_lifecycle_job(&actor, &id(&root)?, &id(&job)?)
            .await?,
    ))
}
async fn resume_job(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, job)): Path<(String, String)>,
) -> Result<Json<LifecycleJob>, ApiError> {
    Ok(Json(
        engine(&app)?
            .resume_lifecycle_job(&actor, &id(&root)?, &id(&job)?)
            .await?,
    ))
}
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<
        Json<PreviewChangeRequest>,
        simple_server::axum::extract::rejection::JsonRejection,
    >,
) -> Result<Json<ChangePreview>, ApiError> {
    Ok(Json(
        app.writer
            .preview_change(
                &actor,
                &id(&root)?,
                &body.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn inspect(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, preview)): Path<(String, String)>,
) -> Result<Json<ChangePreview>, ApiError> {
    Ok(Json(
        app.writer
            .inspect_preview(&actor, &id(&root)?, &id(&preview)?)
            .await?,
    ))
}
async fn advance(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, preview)): Path<(String, String)>,
) -> Result<Json<ChangePreview>, ApiError> {
    Ok(Json(
        app.writer
            .advance_preview(&actor, &id(&root)?, &id(&preview)?)
            .await?,
    ))
}
async fn versions(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, preview)): Path<(String, String)>,
    query: Result<Query<ListRequest>, simple_server::axum::extract::rejection::QueryRejection>,
) -> Result<Json<PreviewVersionPage>, ApiError> {
    Ok(Json(
        app.writer
            .preview_versions(
                &actor,
                &id(&root)?,
                &id(&preview)?,
                &query.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
