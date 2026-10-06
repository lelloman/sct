use crate::api::{ApiError, App, Auth};
use crate::web::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post, put},
};
use sct_protocol::*;
use simple_server::extract::RejectionResponse;
type Result<T> = std::result::Result<T, ApiError>;
type Body<T> = std::result::Result<Json<T>, RejectionResponse>;
type List = std::result::Result<Query<ListRequest>, RejectionResponse>;
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
        .route("/api/v1/admin/backends", get(backends))
        .route("/api/v1/admin/backends/{backend}", put(configure_backend))
        .route(
            "/api/v1/admin/backends/{backend}/configuration",
            get(backend_configuration),
        )
        .route("/api/v1/admin/pools", get(pools).post(create_pool))
        .route("/api/v1/admin/pools/{pool}", patch(update_pool))
        .route("/api/v1/admin/allocations", post(create_allocation))
        .route(
            "/api/v1/admin/allocations/{allocation}",
            put(resize_allocation),
        )
        .route(
            "/api/v1/admin/allocations/{allocation}/management",
            get(allocation_management),
        )
}
async fn backends(State(app): State<App>, Auth(actor): Auth, q: List) -> Result<Json<BackendPage>> {
    Ok(Json(app.writer.list_backends(&actor, &list(q)?).await?))
}
async fn backend_configuration(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(backend): Path<String>,
) -> Result<Json<BackendConfiguration>> {
    Ok(Json(
        app.writer
            .backend_configuration(&actor, &id(&backend)?)
            .await?,
    ))
}
async fn configure_backend(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(backend): Path<String>,
    b: Body<ConfigureBackendRequest>,
) -> Result<Json<BackendConfiguration>> {
    let request = body(b)?;
    if request.inspection.id != id::<BackendId>(&backend)? {
        return Err(ApiError(ErrorCode::InvalidRequest));
    }
    Ok(Json(match &app.transfer {
        Some(engine) => {
            engine
                .configure_backend(&actor, &app.registry, &request)
                .await?
        }
        None => {
            app.writer
                .configure_backend(&actor, &app.registry, &request)
                .await?
        }
    }))
}
async fn pools(State(app): State<App>, Auth(actor): Auth, q: List) -> Result<Json<PoolPage>> {
    Ok(Json(app.writer.list_pools(&actor, &list(q)?).await?))
}
async fn create_pool(
    State(app): State<App>,
    Auth(actor): Auth,
    b: Body<CreatePoolRequest>,
) -> Result<Json<PoolInspection>> {
    Ok(Json(app.writer.create_pool(&actor, &body(b)?).await?))
}
async fn update_pool(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(pool): Path<String>,
    b: Body<UpdatePoolRequest>,
) -> Result<Json<PoolInspection>> {
    Ok(Json(
        app.writer
            .update_pool(&actor, &id(&pool)?, &body(b)?)
            .await?,
    ))
}
async fn create_allocation(
    State(app): State<App>,
    Auth(actor): Auth,
    b: Body<CreateAllocationRequest>,
) -> Result<Json<AllocationManagement>> {
    Ok(Json(app.writer.create_allocation(&actor, &body(b)?).await?))
}
async fn resize_allocation(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(allocation): Path<String>,
    b: Body<ResizeAllocationRequest>,
) -> Result<Json<AllocationManagement>> {
    Ok(Json(
        app.writer
            .resize_allocation(&actor, &id(&allocation)?, &body(b)?)
            .await?,
    ))
}
async fn allocation_management(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(allocation): Path<String>,
) -> Result<Json<AllocationManagement>> {
    Ok(Json(
        app.writer
            .allocation_management(&actor, &id(&allocation)?)
            .await?,
    ))
}
