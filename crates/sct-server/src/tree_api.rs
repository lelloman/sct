use crate::api::{ApiError, App, Auth};
use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
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
        .route("/api/v1/roots/{root}/entries/{entry}/hide", post(hide))
        .route(
            "/api/v1/roots/{root}/entries/{entry}/snapshot",
            get(current),
        )
        .route(
            "/api/v1/roots/{root}/snapshots/{version}/entries",
            get(entries),
        )
        .route("/api/v1/roots/{root}/snapshots/{version}/entry", get(entry))
        .route(
            "/api/v1/roots/{root}/snapshots/{version}/manifest",
            get(manifest),
        )
        .route(
            "/api/v1/roots/{root}/snapshots/{version}/content",
            get(content),
        )
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
        .route(
            "/api/v1/roots/{root}/tree-drafts/{draft}/commit",
            post(commit),
        )
}
async fn hide(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, entry)): Path<(String, String)>,
    body: Result<Json<HideEntryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<EntryInspection>, ApiError> {
    let body = body.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    Ok(Json(
        app.writer
            .hide_entry(&actor, &id(&root)?, &id(&entry)?, &body)
            .await?,
    ))
}
async fn current(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, entry)): Path<(String, String)>,
) -> Result<Json<VersionId>, ApiError> {
    Ok(Json(
        app.writer
            .current_snapshot(&actor, &id(&root)?, &id(&entry)?)
            .await?,
    ))
}
async fn entries(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
    query: Result<Query<SnapshotListQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<SnapshotEntryPage>, ApiError> {
    let query = query.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    Ok(Json(
        app.writer
            .snapshot_entries(&actor, &id(&root)?, &id(&version)?, &query)
            .await?,
    ))
}
async fn entry(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
    query: Result<Query<SnapshotPathQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<DirectoryEntry>, ApiError> {
    let query = query.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    let path: Vec<EntryName> = query.path.as_str()[1..]
        .split('/')
        .map(id)
        .collect::<Result<_, _>>()?;
    Ok(Json(
        app.writer
            .snapshot_entry(&actor, &id(&root)?, &id(&version)?, &path)
            .await?,
    ))
}
async fn manifest(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
    query: Result<Query<SnapshotPathQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<FileManifest>, ApiError> {
    let query = query.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    Ok(Json(
        engine(&app)?
            .snapshot_file_manifest(&actor, &id(&root)?, &id(&version)?, &query.path)
            .await?,
    ))
}
async fn content(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, version)): Path<(String, String)>,
    query: Result<Query<SnapshotPathQuery>, axum::extract::rejection::QueryRejection>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    let query = query.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0;
    crate::transfer_api::content_response(
        engine(&app)?,
        &actor,
        id(&root)?,
        id(&version)?,
        Some(query.path),
        headers,
    )
    .await
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
async fn commit(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, draft)): Path<(String, String)>,
) -> Result<Json<CommitReceipt>, ApiError> {
    let root = id(&root)?;
    let draft = id(&draft)?;
    engine(&app)?
        .tree_draft_status(&actor, &root, &draft)
        .await?;
    Ok(Json(
        engine(&app)?.commit_upload(&actor, &root, &draft).await?,
    ))
}
