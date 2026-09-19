use crate::api::{ApiError, App, Auth};
use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
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
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/roots/{root}/tree-drafts", post(create))
        .route("/api/v1/roots/{root}/tree-drafts/{draft}", get(inspect))
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/identities",
            post(identities),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/mutations",
            post(mutations),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/pages/{manifest}",
            put(page),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/advance",
            post(advance),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/freeze",
            post(freeze),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/abort",
            post(abort),
        )
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/admit",
            post(admit),
        )
}
async fn create(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    body: Result<Json<CreateTreeDraftRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .create_tree_draft(
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
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .tree_draft_status(&actor, &id(&root)?, &id(&draft)?)
            .await?,
    ))
}
async fn identities(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
    body: Result<Json<AllocateDraftIdentitiesRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<DraftIdentities>, ApiError> {
    Ok(Json(
        engine(&app)?
            .allocate_draft_identities(
                &actor,
                &id(&root)?,
                &id(&draft)?,
                &body.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn bounded_body(request: Request) -> Result<axum::body::Bytes, ApiError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        axum::body::to_bytes(request.into_body(), MANIFEST_BYTES),
    )
    .await
    .map_err(|_| ApiError(ErrorCode::DestinationUnavailable))?
    .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
async fn mutations(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
    request: Request,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    let root = id(&root)?;
    let draft = id(&draft)?;
    engine(&app)?
        .tree_draft_status(&actor, &root, &draft)
        .await?;
    let body = bounded_body(request).await?;
    let mutations =
        serde_json::from_slice(&body).map_err(|_| ApiError(ErrorCode::InvalidRequest))?;
    Ok(Json(
        engine(&app)?
            .mutate_tree_draft(&actor, &root, &draft, &mutations)
            .await?,
    ))
}
async fn page(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft, manifest)): Path<(String, String, String)>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let root = id(&root)?;
    let draft = id(&draft)?;
    let manifest = id(&manifest)?;
    engine(&app)?
        .tree_draft_status(&actor, &root, &draft)
        .await?;
    let body = bounded_body(request).await?;
    engine(&app)?
        .tree_draft_page(&actor, &root, &draft, &manifest, &body)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn advance(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .advance_tree_draft(&actor, &id(&root)?, &id(&draft)?)
            .await?,
    ))
}
async fn freeze(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .freeze_tree_draft(&actor, &id(&root)?, &id(&draft)?)
            .await?,
    ))
}
async fn abort(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .abort_tree_draft(&actor, &id(&root)?, &id(&draft)?)
            .await?,
    ))
}
async fn admit(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<TreeDraftStatus>, ApiError> {
    Ok(Json(
        engine(&app)?
            .admit_tree_draft(&actor, &id(&root)?, &id(&draft)?)
            .await?,
    ))
}
