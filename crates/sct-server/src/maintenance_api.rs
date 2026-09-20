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
type Body<T> = std::result::Result<Json<T>, JsonRejection>;
type List = std::result::Result<Query<ListRequest>, QueryRejection>;
fn id<T: std::str::FromStr>(s: &str) -> Result<T> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn body<T>(v: Body<T>) -> Result<T> {
    Ok(v.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
fn list(v: List) -> Result<ListRequest> {
    Ok(v.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/admin/maintenance", get(all).post(start))
        .route("/api/v1/admin/maintenance/{job}/resume", post(resume))
        .route("/api/v1/admin/roots/{root}/repair", post(repair))
        .route("/api/v1/roots/{root}/maintenance", get(root_jobs))
        .route("/api/v1/roots/{root}/maintenance/{job}", get(job))
        .route("/api/v1/roots/{root}/integrity-findings", get(findings))
}
async fn repair(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    b: Body<StartMaintenanceRequest>,
) -> Result<Json<MaintenanceJob>> {
    app.writer.require_admin(&actor).await?;
    let request = body(b)?;
    if request.kind != MaintenanceKind::MirrorRepair || request.root_id != id::<RootId>(&root)? {
        return Err(ApiError(ErrorCode::InvalidRequest));
    }
    Ok(Json(app.writer.start_maintenance(&actor, &request).await?))
}
async fn start(
    State(app): State<App>,
    Auth(actor): Auth,
    b: Body<StartMaintenanceRequest>,
) -> Result<Json<MaintenanceJob>> {
    let request = body(b)?;
    app.writer.require_admin(&actor).await?;
    if request.kind != MaintenanceKind::IntegrityScan {
        return Err(ApiError(ErrorCode::UnsupportedOperation));
    }
    Ok(Json(app.writer.start_maintenance(&actor, &request).await?))
}
async fn resume(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(job): Path<String>,
    b: Body<ExpectedRevision>,
) -> Result<Json<MaintenanceJob>> {
    Ok(Json(
        app.writer
            .resume_maintenance(&actor, &id(&job)?, &body(b)?.expected_revision)
            .await?,
    ))
}
async fn all(State(app): State<App>, Auth(actor): Auth, q: List) -> Result<Json<MaintenancePage>> {
    Ok(Json(
        app.writer.list_maintenance(&actor, None, &list(q)?).await?,
    ))
}
async fn root_jobs(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    q: List,
) -> Result<Json<MaintenancePage>> {
    Ok(Json(
        app.writer
            .list_maintenance(&actor, Some(&id(&root)?), &list(q)?)
            .await?,
    ))
}
async fn job(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((root, job)): Path<(String, String)>,
) -> Result<Json<MaintenanceJob>> {
    let result = app.writer.maintenance_job(&actor, &id(&job)?).await?;
    if result.root_id != id::<RootId>(&root)? {
        return Err(ApiError(ErrorCode::NotFound));
    }
    Ok(Json(result))
}
async fn findings(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    q: List,
) -> Result<Json<IntegrityFindingPage>> {
    Ok(Json(
        app.writer
            .list_integrity_findings(&actor, &id(&root)?, &list(q)?)
            .await?,
    ))
}
