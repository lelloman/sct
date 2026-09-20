use crate::api::{ApiError, App, Auth};
use sct_protocol::*;
use simple_server::axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::QueryRejection},
    routing::get,
};
type Result<T> = std::result::Result<T, ApiError>;
type List = std::result::Result<Query<ListRequest>, QueryRejection>;
fn id<T: std::str::FromStr>(s: &str) -> Result<T> {
    s.parse().map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn list(v: List) -> Result<ListRequest> {
    Ok(v.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/roots/{root}/usage", get(usage))
        .route("/api/v1/roots/{root}/sessions", get(sessions))
        .route("/api/v1/roots/{root}/jobs", get(root_jobs))
        .route("/api/v1/roots/{root}/audit", get(root_audit))
        .route("/api/v1/admin/jobs", get(jobs))
        .route("/api/v1/admin/audit", get(audit))
}
async fn usage(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
) -> Result<Json<RootCapacityUsage>> {
    Ok(Json(app.writer.root_usage(&actor, &id(&root)?).await?))
}
async fn sessions(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    q: List,
) -> Result<Json<TransferActivityPage>> {
    Ok(Json(
        app.writer
            .list_transfer_activity(&actor, &id(&root)?, &list(q)?)
            .await?,
    ))
}
async fn root_jobs(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    q: List,
) -> Result<Json<JobActivityPage>> {
    Ok(Json(
        app.writer
            .list_job_activity(&actor, Some(&id(&root)?), &list(q)?)
            .await?,
    ))
}
async fn root_audit(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    q: List,
) -> Result<Json<AuditPage>> {
    Ok(Json(
        app.writer
            .list_audit(&actor, Some(&id(&root)?), &list(q)?)
            .await?,
    ))
}
async fn jobs(State(app): State<App>, Auth(actor): Auth, q: List) -> Result<Json<JobActivityPage>> {
    Ok(Json(
        app.writer
            .list_job_activity(&actor, None, &list(q)?)
            .await?,
    ))
}
async fn audit(State(app): State<App>, Auth(actor): Auth, q: List) -> Result<Json<AuditPage>> {
    Ok(Json(app.writer.list_audit(&actor, None, &list(q)?).await?))
}
