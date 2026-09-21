use crate::oidc::Oidc;
use sct_core::{auth::Actor, backend::Registry, error::Error, lease::Writer};
use sct_protocol::*;
use simple_server::axum::{
    Json, Router,
    extract::Request,
    extract::{FromRequestParts, Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct App {
    pub transfer: Option<sct_core::transfer_io::TransferEngine>,
    pub writer: Writer,
    pub registry: Registry,
    pub oidc: Option<Arc<Oidc>>,
    pub public_url: String,
    pub secure_cookies: bool,
}
pub struct ApiError(pub ErrorCode);
impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e.0)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = crate::correlation::request_id();
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ErrorResponse::new(request_id.parse().unwrap(), self.0.clone());
        let retry = matches!(body.retry, RetryGuidance::After { .. });
        let mut response = (status, Json(body)).into_response();
        response
            .headers_mut()
            .insert("x-request-id", request_id.parse().unwrap());
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        response.extensions_mut().insert(self.0);
        if retry {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "1".parse().unwrap());
        }
        response
    }
}
type Result<T> = std::result::Result<T, ApiError>;
pub struct Auth(pub Actor);
impl FromRequestParts<App> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &App) -> Result<Self> {
        if let Some(auth) = parts.headers.get(header::AUTHORIZATION) {
            let token = auth
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("Bearer "))
                .ok_or(ApiError(ErrorCode::Unauthorized))?;
            return Ok(Self(state.writer.authenticate(token, false, None).await?));
        }
        let token =
            cookie(&parts.headers, session_name(state)).ok_or(ApiError(ErrorCode::Unauthorized))?;
        let mutation = !matches!(
            parts.method,
            simple_server::axum::http::Method::GET
                | simple_server::axum::http::Method::HEAD
                | simple_server::axum::http::Method::OPTIONS
        );
        let csrf = if mutation {
            let origin = parts
                .headers
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok());
            if origin != Some(state.public_url.trim_end_matches('/')) {
                return Err(ApiError(ErrorCode::Forbidden));
            }
            Some(
                parts
                    .headers
                    .get("x-csrf-token")
                    .and_then(|v| v.to_str().ok())
                    .ok_or(ApiError(ErrorCode::Forbidden))?,
            )
        } else {
            None
        };
        Ok(Self(state.writer.authenticate(token, true, csrf).await?))
    }
}
pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
}
fn session_name(state: &App) -> &'static str {
    if state.secure_cookies {
        "__Host-sct_session"
    } else {
        "sct_session"
    }
}
fn csrf_name(state: &App) -> &'static str {
    if state.secure_cookies {
        "__Host-sct_csrf"
    } else {
        "sct_csrf"
    }
}
fn flow_name(state: &App) -> &'static str {
    if state.secure_cookies {
        "__Host-sct_login"
    } else {
        "sct_login"
    }
}
fn set_cookie(
    response: &mut Response,
    name: &str,
    value: &str,
    age: u32,
    http_only: bool,
    secure: bool,
) {
    let value = format!(
        "{name}={value}; Path=/; Max-Age={age}; SameSite=Lax{}{}",
        if http_only { "; HttpOnly" } else { "" },
        if secure { "; Secure" } else { "" }
    );
    response
        .headers_mut()
        .append(header::SET_COOKIE, value.parse().expect("generated cookie"));
}
fn body<T>(value: std::result::Result<Json<T>, JsonRejection>) -> Result<T> {
    value
        .map(|j| j.0)
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn query<T>(
    value: std::result::Result<Query<T>, simple_server::axum::extract::rejection::QueryRejection>,
) -> Result<T> {
    value
        .map(|q| q.0)
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn identifier<T: std::str::FromStr>(value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}

pub fn routes(state: App) -> Router {
    Router::new()
        .route("/health/ready", get(ready))
        .route("/auth/login", get(login))
        .route("/auth/options", get(login_options))
        .route("/auth/callback", get(callback))
        .route("/auth/session", get(session).post(token_session))
        .route("/auth/logout", post(logout))
        .route("/auth/tokens", post(issue_token))
        .route("/auth/tokens/{credential}", delete(revoke_token))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/roots", get(list_roots).post(create_root))
        .route("/api/v1/roots/{root}", get(inspect_root).patch(update_root))
        .route("/api/v1/roots/{root}/operation", get(operation))
        .route("/api/v1/roots/{root}/resolve", get(resolve))
        .route(
            "/api/v1/roots/{root}/entries",
            get(list_entries).post(create_directory),
        )
        .route("/api/v1/entries/{entry}", get(inspect_entry))
        .route("/api/v1/entries/{entry}/rename", post(rename))
        .route("/api/v1/policies", get(effective_policy).patch(set_policy))
        .route("/api/v1/admin/grants", get(list_grants).post(create_grant))
        .route(
            "/api/v1/admin/grants/{grant}",
            get(inspect_grant).patch(update_grant).delete(revoke_grant),
        )
        .route("/api/v1/admin/allocations", get(list_allocations))
        .route(
            "/api/v1/admin/roots/{root}/policy-constraints",
            simple_server::axum::routing::patch(policy_constraints),
        )
        .route("/api/v1/admin/pools/{pool}", get(pool))
        .route("/api/v1/admin/allocations/{allocation}", get(allocation))
        .route("/api/v1/admin/backends/{backend}", get(backend))
        .route("/api/v1/admin/adapters/{kind}", get(adapter))
        .merge(crate::transfer_api::routes())
        .merge(crate::tree_api::routes())
        .merge(crate::lifecycle_api::routes())
        .merge(crate::management_api::routes())
        .merge(crate::storage_api::routes())
        .merge(crate::inspection_api::routes())
        .merge(crate::maintenance_api::routes())
        .merge(crate::checkpoint_api::routes())
        .layer(simple_server::body_limit::BodyLimit::max(65536))
        .layer(middleware::from_fn(response_headers))
        .with_state(state)
}
async fn response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    // Normalize framework rejections too (invalid UTF-8 path segments, unsupported
    // methods, body limits), without reflecting extractor diagnostics or input.
    if (response.status().is_client_error() || response.status().is_server_error())
        && !response.headers().contains_key("x-request-id")
    {
        let code = match response.status() {
            StatusCode::SERVICE_UNAVAILABLE => ErrorCode::DestinationUnavailable,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                ErrorCode::UnsupportedOperation
            }
            status if status.is_client_error() => ErrorCode::InvalidRequest,
            _ => ErrorCode::InternalError,
        };
        response = ApiError(code).into_response();
    }
    if !response.headers().contains_key("x-request-id") {
        response.headers_mut().insert(
            "x-request-id",
            crate::correlation::request_id().parse().unwrap(),
        );
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
}
async fn ready(State(s): State<App>) -> StatusCode {
    if s.writer.ready().await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
async fn capabilities(Auth(_): Auth) -> Json<Capabilities> {
    Json(crate::capabilities())
}
async fn list_roots(
    State(s): State<App>,
    Auth(a): Auth,
    q: std::result::Result<
        Query<ListRequest>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<RootPage>> {
    Ok(Json(s.writer.list_roots(&a, &query(q)?).await?))
}
async fn create_root(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<CreateRootRequest>, JsonRejection>,
) -> Result<Json<RootInspection>> {
    Ok(Json(s.writer.create_root(&a, &body(b)?).await?))
}
async fn inspect_root(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
) -> Result<Json<RootInspection>> {
    Ok(Json(s.writer.inspect_root(&a, &identifier(&root)?).await?))
}
async fn update_root(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    b: std::result::Result<Json<UpdateRootRequest>, JsonRejection>,
) -> Result<Json<RootInspection>> {
    Ok(Json(
        s.writer
            .update_root(&a, &identifier(&root)?, &body(b)?)
            .await?,
    ))
}
async fn operation(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
) -> Result<Json<OperationInspection>> {
    Ok(Json(
        s.writer.inspect_operation(&a, &identifier(&root)?).await?,
    ))
}
async fn resolve(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    q: std::result::Result<
        Query<ResolvePathRequest>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<EntryInspection>> {
    Ok(Json(
        s.writer
            .resolve_path(&a, &identifier(&root)?, &query(q)?.path)
            .await?,
    ))
}
async fn create_directory(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    b: std::result::Result<Json<CreateDirectoryRequest>, JsonRejection>,
) -> Result<Json<EntryInspection>> {
    Ok(Json(
        s.writer
            .create_directory(&a, &identifier(&root)?, &body(b)?)
            .await?,
    ))
}
async fn list_entries(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    q: std::result::Result<
        Query<ListEntriesQuery>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<EntryPage>> {
    let q = query(q)?;
    let cursor = match (q.cursor, q.directory_revision) {
        (Some(token), Some(directory_revision)) => Some(DirectoryCursor {
            token,
            directory_revision,
            directory_id: q.directory_id.clone(),
        }),
        (None, None) => None,
        _ => return Err(ApiError(ErrorCode::InvalidRequest)),
    };
    Ok(Json(
        s.writer
            .list_entries(
                &a,
                &identifier(&root)?,
                &q.directory_id,
                &ListEntriesRequest {
                    limit: q.limit,
                    cursor,
                    include_hidden: q.include_hidden,
                },
            )
            .await?,
    ))
}
async fn inspect_entry(
    State(s): State<App>,
    Auth(a): Auth,
    Path(entry): Path<String>,
) -> Result<Json<EntryInspection>> {
    Ok(Json(
        s.writer.inspect_entry(&a, &identifier(&entry)?).await?,
    ))
}
async fn rename(
    State(s): State<App>,
    Auth(a): Auth,
    Path(entry): Path<String>,
    b: std::result::Result<Json<RenameEntryRequest>, JsonRejection>,
) -> Result<Json<EntryInspection>> {
    Ok(Json(
        s.writer
            .rename_entry(&a, &identifier(&entry)?, &body(b)?)
            .await?,
    ))
}
async fn effective_policy(
    State(s): State<App>,
    Auth(a): Auth,
    q: std::result::Result<
        Query<PolicyScope>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<EffectivePolicy>> {
    Ok(Json(s.writer.effective_policy(&a, &query(q)?).await?))
}
async fn set_policy(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<SetPolicyRequest>, JsonRejection>,
) -> Result<Json<EffectivePolicy>> {
    Ok(Json(s.writer.set_policy(&a, &body(b)?).await?))
}
async fn create_grant(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<CreateGrantRequest>, JsonRejection>,
) -> Result<Json<GrantInspection>> {
    Ok(Json(s.writer.create_grant(&a, &body(b)?).await?))
}
async fn inspect_grant(
    State(s): State<App>,
    Auth(a): Auth,
    Path(grant): Path<String>,
) -> Result<Json<GrantInspection>> {
    Ok(Json(
        s.writer.inspect_grant(&a, &identifier(&grant)?).await?,
    ))
}
async fn update_grant(
    State(s): State<App>,
    Auth(a): Auth,
    Path(grant): Path<String>,
    b: std::result::Result<Json<UpdateGrantRequest>, JsonRejection>,
) -> Result<Json<GrantInspection>> {
    Ok(Json(
        s.writer
            .update_grant(&a, &identifier(&grant)?, &body(b)?)
            .await?,
    ))
}
async fn revoke_grant(
    State(s): State<App>,
    Auth(a): Auth,
    Path(grant): Path<String>,
    q: std::result::Result<
        Query<ExpectedRevision>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<StatusCode> {
    s.writer
        .revoke_grant(&a, &identifier(&grant)?, &query(q)?.expected_revision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_grants(
    State(s): State<App>,
    Auth(a): Auth,
    q: std::result::Result<
        Query<ListGrantsQuery>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<GrantPage>> {
    let q = query(q)?;
    Ok(Json(
        s.writer
            .list_grants(
                &a,
                &q.root_id,
                &ListRequest {
                    limit: q.limit,
                    cursor: q.cursor,
                },
            )
            .await?,
    ))
}
async fn pool(
    State(s): State<App>,
    Auth(a): Auth,
    Path(pool): Path<String>,
) -> Result<Json<PoolInspection>> {
    Ok(Json(s.writer.inspect_pool(&a, &identifier(&pool)?).await?))
}
async fn allocation(
    State(s): State<App>,
    Auth(a): Auth,
    Path(allocation): Path<String>,
) -> Result<Json<AllocationInspection>> {
    Ok(Json(
        s.writer
            .inspect_allocation(&a, &identifier(&allocation)?)
            .await?,
    ))
}
async fn backend(
    State(s): State<App>,
    Auth(a): Auth,
    Path(backend): Path<String>,
) -> Result<Json<BackendInspection>> {
    Ok(Json(
        s.writer.inspect_backend(&a, &identifier(&backend)?).await?,
    ))
}
async fn adapter(
    State(s): State<App>,
    Auth(a): Auth,
    Path(kind): Path<String>,
) -> Result<Json<AdapterDocumentation>> {
    s.writer.require_admin(&a).await?;
    Ok(Json(s.registry.get(&kind)?.documentation()))
}
async fn login(State(s): State<App>) -> Result<Response> {
    let oidc = s
        .oidc
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))?;
    let (url, browser) = oidc.begin(&s.writer).await?;
    let mut response = Redirect::to(&url).into_response();
    set_cookie(
        &mut response,
        flow_name(&s),
        &browser,
        600,
        true,
        s.secure_cookies,
    );
    Ok(response)
}
#[derive(serde::Deserialize)]
struct Callback {
    state: String,
    code: String,
}
async fn callback(
    State(s): State<App>,
    headers: HeaderMap,
    q: std::result::Result<
        Query<Callback>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Response> {
    let q = query(q)?;
    let oidc = s
        .oidc
        .as_ref()
        .ok_or(ApiError(ErrorCode::UnsupportedOperation))?;
    let browser = cookie(&headers, flow_name(&s)).ok_or(ApiError(ErrorCode::Unauthorized))?;
    let issued = oidc.finish(&s.writer, &q.state, browser, &q.code).await?;
    let mut response = Redirect::to("/").into_response();
    set_cookie(&mut response, flow_name(&s), "", 0, true, s.secure_cookies);
    set_cookie(
        &mut response,
        session_name(&s),
        &issued.secret,
        43200,
        true,
        s.secure_cookies,
    );
    set_cookie(
        &mut response,
        csrf_name(&s),
        issued.csrf.as_deref().unwrap(),
        43200,
        false,
        s.secure_cookies,
    );
    Ok(response)
}
async fn session(Auth(a): Auth) -> Result<Json<SessionInspection>> {
    Ok(Json(SessionInspection {
        principal_id: identifier(a.principal_id())?,
    }))
}
async fn login_options(State(s): State<App>) -> Json<BrowserLoginOptions> {
    Json(BrowserLoginOptions {
        oidc_enabled: s.oidc.is_some(),
    })
}
async fn token_session(
    State(s): State<App>,
    Auth(a): Auth,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        != Some(s.public_url.trim_end_matches('/'))
        || !headers.contains_key(header::AUTHORIZATION)
    {
        return Err(ApiError(ErrorCode::Forbidden));
    }
    let issued = s.writer.browser_session(&a).await?;
    let mut response = Json(SessionInspection {
        principal_id: identifier(a.principal_id())?,
    })
    .into_response();
    set_cookie(
        &mut response,
        session_name(&s),
        &issued.secret,
        43200,
        true,
        s.secure_cookies,
    );
    set_cookie(
        &mut response,
        csrf_name(&s),
        issued.csrf.as_deref().unwrap(),
        43200,
        false,
        s.secure_cookies,
    );
    Ok(response)
}
async fn logout(State(s): State<App>, Auth(a): Auth, headers: HeaderMap) -> Result<Response> {
    let token = cookie(&headers, session_name(&s)).ok_or(ApiError(ErrorCode::Unauthorized))?;
    s.writer.revoke_session(&a, token).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(
        &mut response,
        session_name(&s),
        "",
        0,
        true,
        s.secure_cookies,
    );
    set_cookie(&mut response, csrf_name(&s), "", 0, false, s.secure_cookies);
    Ok(response)
}
async fn issue_token(State(s): State<App>, Auth(a): Auth) -> Result<Json<IssuedToken>> {
    let issued = s
        .writer
        .issue_token(&a, &identifier(a.principal_id())?)
        .await?;
    Ok(Json(IssuedToken {
        id: identifier(&issued.id)?,
        secret: issued.secret,
    }))
}
async fn revoke_token(
    State(s): State<App>,
    Auth(a): Auth,
    Path(credential): Path<String>,
) -> Result<StatusCode> {
    let id: CredentialId = identifier(&credential)?;
    s.writer.revoke_credential(&a, id.as_str()).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_allocations(
    State(s): State<App>,
    Auth(a): Auth,
    q: std::result::Result<
        Query<ListRequest>,
        simple_server::axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<AllocationPage>> {
    Ok(Json(s.writer.list_allocations(&a, &query(q)?).await?))
}
async fn policy_constraints(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    b: std::result::Result<Json<PolicyConstraintsRequest>, JsonRejection>,
) -> Result<Json<RootInspection>> {
    let request = body(b)?;
    let root = identifier(&root)?;
    s.writer
        .set_policy_constraints(
            &a,
            &root,
            &request.expected_revision,
            request.min_versions,
            request.max_versions,
            request.allow_relaxed,
        )
        .await?;
    Ok(Json(s.writer.inspect_root(&a, &root).await?))
}
