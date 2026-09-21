use crate::api::{ApiError, App, Auth};
use sct_protocol::*;
use simple_server::axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    routing::{get, post},
};
type Result<T> = std::result::Result<T, ApiError>;
fn id<T: std::str::FromStr>(s: &str) -> Result<T> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/archives", get(collections).post(create_collection))
        .route(
            "/api/v1/archives/{collection}",
            get(collection).patch(update_collection),
        )
        .route("/api/v1/archives/usage", get(usage))
        .route("/api/v1/archives/{collection}/sets", get(sets).post(plan))
        .route("/api/v1/archives/{collection}/sets/{set}", get(set))
        .route("/api/v1/archives/{collection}/sets/{set}/discs", get(discs))
        .route(
            "/api/v1/archives/{collection}/sets/{set}/entries",
            get(entries),
        )
        .route(
            "/api/v1/archives/{collection}/discs/{disc}/objects",
            get(objects),
        )
        .route(
            "/api/v1/archives/{collection}/sets/{set}/confirm",
            post(confirm),
        )
        .route(
            "/api/v1/archives/{collection}/sets/{set}/cancel",
            post(cancel),
        )
}
async fn collections(
    State(app): State<App>,
    Auth(actor): Auth,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveCollectionPage>> {
    Ok(Json(
        app.writer
            .list_archive_collections(
                &actor,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn collection(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(collection): Path<String>,
) -> Result<Json<ArchiveCollection>> {
    Ok(Json(
        app.writer
            .inspect_archive_collection(&actor, &id(&collection)?)
            .await?,
    ))
}
async fn usage(State(app): State<App>, Auth(actor): Auth) -> Result<Json<ArchiveUsage>> {
    Ok(Json(app.writer.archive_usage(&actor).await?))
}
async fn sets(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(collection): Path<String>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveSetPage>> {
    Ok(Json(
        app.writer
            .list_archive_sets(
                &actor,
                &id(&collection)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn set(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, set)): Path<(String, String)>,
) -> Result<Json<ArchiveSet>> {
    Ok(Json(
        app.writer
            .inspect_archive_set(&actor, &id(&collection)?, &id(&set)?)
            .await?,
    ))
}
async fn discs(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, set)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveDiscPage>> {
    Ok(Json(
        app.writer
            .list_archive_discs(
                &actor,
                &id(&collection)?,
                &id(&set)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn entries(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, set)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveEntryPage>> {
    Ok(Json(
        app.writer
            .list_archive_entries(
                &actor,
                &id(&collection)?,
                &id(&set)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn objects(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, disc)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveObjectPage>> {
    Ok(Json(
        app.writer
            .list_archive_objects(
                &actor,
                &id(&collection)?,
                &id(&disc)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn create_collection(
    State(app): State<App>,
    Auth(actor): Auth,
    request: std::result::Result<Json<CreateArchiveCollectionRequest>, JsonRejection>,
) -> Result<Json<ArchiveCollection>> {
    Ok(Json(
        app.writer
            .create_archive_collection(
                &actor,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn update_collection(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(collection): Path<String>,
    request: std::result::Result<Json<UpdateArchiveCollectionRequest>, JsonRejection>,
) -> Result<Json<ArchiveCollection>> {
    Ok(Json(
        app.writer
            .update_archive_collection(
                &actor,
                &id(&collection)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn plan(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(collection): Path<String>,
    request: std::result::Result<Json<CreateArchivePlanRequest>, JsonRejection>,
) -> Result<Json<ArchiveSet>> {
    Ok(Json(
        app.writer
            .create_archive_plan(
                &actor,
                &id(&collection)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn confirm(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, set)): Path<(String, String)>,
    request: std::result::Result<Json<ArchiveSetActionRequest>, JsonRejection>,
) -> Result<Json<ArchiveSet>> {
    Ok(Json(
        app.writer
            .confirm_archive_set(
                &actor,
                &id(&collection)?,
                &id(&set)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn cancel(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, set)): Path<(String, String)>,
    request: std::result::Result<Json<ArchiveSetActionRequest>, JsonRejection>,
) -> Result<Json<ArchiveSet>> {
    Ok(Json(
        app.writer
            .cancel_archive_set(
                &actor,
                &id(&collection)?,
                &id(&set)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
