use crate::api::{ApiError, App, Auth};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use sct_protocol::*;
fn id<T: std::str::FromStr>(s: &str) -> Result<T, ApiError> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
pub fn routes() -> Router<App> {
    Router::new()
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
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<Json<PreviewChangeRequest>, axum::extract::rejection::JsonRejection>,
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
    query: Result<Query<ListRequest>, axum::extract::rejection::QueryRejection>,
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
