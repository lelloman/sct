use crate::api::{ApiError, App, Auth};
use sct_protocol::*;
use simple_server::axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    routing::{delete, get, patch},
};
type Result<T> = std::result::Result<T, ApiError>;
type Body<T> = std::result::Result<Json<T>, JsonRejection>;
type List = std::result::Result<Query<ListRequest>, QueryRejection>;
fn id<T: std::str::FromStr>(value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn body<T>(value: Body<T>) -> Result<T> {
    Ok(value.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
fn list(value: List) -> Result<ListRequest> {
    Ok(value.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/api/v1/account", get(account))
        .route("/api/v1/credentials/{credential}", delete(revoke))
        .route("/api/v1/principals", get(principals).post(create_principal))
        .route(
            "/api/v1/principals/{principal}",
            get(principal).patch(update_principal),
        )
        .route(
            "/api/v1/principals/{principal}/credentials",
            get(credentials).post(issue),
        )
        .route(
            "/api/v1/roots/{root}/management",
            get(root).patch(update_root),
        )
        .route("/api/v1/roots/{root}/quota-ceiling", patch(ceiling))
}
async fn account(State(app): State<App>, Auth(actor): Auth) -> Result<Json<AccountInspection>> {
    Ok(Json(app.writer.account(&actor).await?))
}
async fn principals(
    State(app): State<App>,
    Auth(actor): Auth,
    q: List,
) -> Result<Json<PrincipalPage>> {
    Ok(Json(app.writer.list_principals(&actor, &list(q)?).await?))
}
async fn create_principal(
    State(app): State<App>,
    Auth(actor): Auth,
    b: Body<CreatePrincipalRequest>,
) -> Result<Json<PrincipalInspection>> {
    let request = body(b)?;
    app.writer.create_principal(&actor, &request.id).await?;
    Ok(Json(
        app.writer.inspect_principal(&actor, &request.id).await?,
    ))
}
async fn principal(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(target): Path<String>,
) -> Result<Json<PrincipalInspection>> {
    Ok(Json(
        app.writer.inspect_principal(&actor, &id(&target)?).await?,
    ))
}
async fn update_principal(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(target): Path<String>,
    b: Body<UpdatePrincipalRequest>,
) -> Result<Json<PrincipalInspection>> {
    Ok(Json(
        app.writer
            .update_principal(&actor, &id(&target)?, &body(b)?)
            .await?,
    ))
}
async fn credentials(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(target): Path<String>,
    q: List,
) -> Result<Json<CredentialPage>> {
    Ok(Json(
        app.writer
            .list_credentials(&actor, &id(&target)?, &list(q)?)
            .await?,
    ))
}
async fn issue(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(target): Path<String>,
) -> Result<Json<IssuedToken>> {
    let issued = app.writer.issue_token(&actor, &id(&target)?).await?;
    Ok(Json(IssuedToken {
        id: id(&issued.id)?,
        secret: issued.secret,
    }))
}
async fn root(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
) -> Result<Json<RootManagement>> {
    Ok(Json(app.writer.root_management(&actor, &id(&root)?).await?))
}
async fn update_root(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    b: Body<UpdateRootManagementRequest>,
) -> Result<Json<RootManagement>> {
    Ok(Json(
        app.writer
            .update_root_management(&actor, &id(&root)?, &body(b)?)
            .await?,
    ))
}
async fn ceiling(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(root): Path<String>,
    b: Body<UpdateQuotaCeilingRequest>,
) -> Result<Json<RootManagement>> {
    Ok(Json(
        app.writer
            .update_quota_ceiling(&actor, &id(&root)?, &body(b)?)
            .await?,
    ))
}

async fn revoke(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(credential): Path<String>,
) -> Result<simple_server::axum::http::StatusCode> {
    let credential: CredentialId = id(&credential)?;
    app.writer
        .revoke_credential(&actor, credential.as_str())
        .await?;
    Ok(simple_server::axum::http::StatusCode::NO_CONTENT)
}
