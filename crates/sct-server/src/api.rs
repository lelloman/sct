use crate::oidc::Oidc;
use sct_core::{auth::Actor, backend::Registry, error::Error, lease::Writer};
use sct_protocol::*;
use simple_server::auth::{AsyncAccess, HeaderCredential, RepeatedHeaders, SchemeCase};
use simple_server::extract::RejectionResponse;
use simple_server::web::{
    Json, Router,
    extract::Request,
    extract::{FromRequestParts, Path, Query, State},
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
        simple_server::extract::IntoRejectionResponse::into_rejection_response(self).into_response()
    }
}
impl simple_server::extract::IntoRejectionResponse for ApiError {
    fn into_rejection_response(self) -> simple_server::extract::RejectionResponse {
        let request_id = crate::correlation::request_id();
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ErrorResponse::new(request_id.parse().unwrap(), self.0.clone());
        let retry = matches!(body.retry, RetryGuidance::After { .. });
        let mut response = simple_server::web::http::Response::new(
            serde_json::to_vec(&body).expect("protocol error serializes"),
        );
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        response
            .headers_mut()
            .insert("x-request-id", request_id.parse().unwrap());
        simple_server::response_headers::replace(
            response.headers_mut(),
            header::CACHE_CONTROL,
            "no-store".parse().unwrap(),
        );
        response.extensions_mut().insert(self.0);
        if retry {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "1".parse().unwrap());
        }
        response.into()
    }
}
type Result<T> = std::result::Result<T, ApiError>;
pub struct Auth(pub Actor);
struct AuthContext {
    writer: Writer,
    secret: String,
    is_session: bool,
    csrf: Option<String>,
    deny_browser_content_mutation: bool,
}

async fn authenticate(context: AuthContext) -> Result<Actor> {
    AsyncAccess::new(|context: &AuthContext| {
        Box::pin(async move {
            context
                .writer
                .authenticate(&context.secret, context.is_session, context.csrf.as_deref())
                .await
                .map_err(ApiError::from)
        })
    })
    .with_check(|_, context| {
        Box::pin(async move {
            if context.deny_browser_content_mutation {
                Err(ApiError(ErrorCode::Forbidden))
            } else {
                Ok(())
            }
        })
    })
    .evaluate(&context)
    .await
}

impl FromRequestParts<App> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &App) -> Result<Self> {
        if parts.headers.contains_key(header::AUTHORIZATION) {
            let token = HeaderCredential::new(header::AUTHORIZATION)
                .with_scheme("Bearer", SchemeCase::Exact)
                .repeated(RepeatedHeaders::First)
                .allow_empty(true)
                .extract(&parts.headers)
                .map_err(|_| ApiError(ErrorCode::Unauthorized))?;
            return Ok(Self(
                authenticate(AuthContext {
                    writer: state.writer.clone(),
                    secret: token.expose().to_owned(),
                    is_session: false,
                    csrf: None,
                    deny_browser_content_mutation: false,
                })
                .await?,
            ));
        }
        let token =
            cookie(&parts.headers, session_name(state)).ok_or(ApiError(ErrorCode::Unauthorized))?;
        let mutation = !matches!(
            parts.method,
            simple_server::web::http::Method::GET
                | simple_server::web::http::Method::HEAD
                | simple_server::web::http::Method::OPTIONS
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
        let route = parts
            .extensions
            .get::<simple_server::web::extract::MatchedPath>()
            .map(|path| path.as_str())
            .unwrap_or(parts.uri.path());
        let actor = authenticate(AuthContext {
            writer: state.writer.clone(),
            secret: token.to_owned(),
            is_session: true,
            csrf: csrf.map(str::to_owned),
            deny_browser_content_mutation: mutation && browser_content_mutation(route),
        })
        .await?;
        Ok(Self(actor))
    }
}
fn browser_content_mutation(route: &str) -> bool {
    route.starts_with("/api/v1/entries/")
        || route
            .strip_prefix("/api/v1/roots/")
            .and_then(|tail| tail.split_once('/'))
            .is_some_and(|(_, tail)| {
                matches!(
                    tail.split('/').next(),
                    Some("entries" | "uploads" | "tree-drafts" | "purge-jobs")
                )
            })
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
fn body<T>(value: std::result::Result<Json<T>, RejectionResponse>) -> Result<T> {
    value
        .map(|j| j.0)
        .map_err(|_| ApiError(ErrorCode::InvalidRequest))
}
fn query<T>(
    value: std::result::Result<Query<T>, simple_server::extract::RejectionResponse>,
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
            simple_server::web::routing::patch(policy_constraints),
        )
        .route("/api/v1/admin/pools/{pool}", get(pool))
        .route("/api/v1/admin/allocations/{allocation}", get(allocation))
        .route("/api/v1/admin/backends/{backend}", get(backend))
        .route("/api/v1/admin/adapters/{kind}", get(adapter))
        .merge(crate::transfer_api::routes())
        .merge(crate::tree_api::routes())
        .merge(crate::lifecycle_api::routes())
        .merge(crate::management_api::routes())
        .merge(crate::archive_api::routes())
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
    simple_server::response_headers::replace(
        response.headers_mut(),
        header::CACHE_CONTROL,
        "no-store".parse().unwrap(),
    );
    simple_server::response_headers::replace(
        response.headers_mut(),
        simple_server::response_headers::HeaderName::from_static("x-content-type-options"),
        "nosniff".parse().unwrap(),
    );
    simple_server::response_headers::replace(
        response.headers_mut(),
        simple_server::response_headers::HeaderName::from_static("referrer-policy"),
        "no-referrer".parse().unwrap(),
    );
    response
}
async fn ready(State(s): State<App>) -> StatusCode {
    let writer = s.writer.clone();
    let ready = simple_server::health::Probe::readiness(simple_server::health::Check::new(
        "writer",
        move || {
            let writer = writer.clone();
            async move { writer.ready().await.then_some(()).ok_or(()) }
        },
    ))
    .run()
    .await
    .is_ok();
    if ready {
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
    q: std::result::Result<Query<ListRequest>, simple_server::extract::RejectionResponse>,
) -> Result<Json<RootPage>> {
    Ok(Json(s.writer.list_roots(&a, &query(q)?).await?))
}
async fn create_root(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<CreateRootRequest>, RejectionResponse>,
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
    b: std::result::Result<Json<UpdateRootRequest>, RejectionResponse>,
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
    q: std::result::Result<Query<ResolvePathRequest>, simple_server::extract::RejectionResponse>,
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
    b: std::result::Result<Json<CreateDirectoryRequest>, RejectionResponse>,
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
    q: std::result::Result<Query<ListEntriesQuery>, simple_server::extract::RejectionResponse>,
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
    b: std::result::Result<Json<RenameEntryRequest>, RejectionResponse>,
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
    q: std::result::Result<Query<PolicyScope>, simple_server::extract::RejectionResponse>,
) -> Result<Json<EffectivePolicy>> {
    Ok(Json(s.writer.effective_policy(&a, &query(q)?).await?))
}
async fn set_policy(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<SetPolicyRequest>, RejectionResponse>,
) -> Result<Json<EffectivePolicy>> {
    Ok(Json(s.writer.set_policy(&a, &body(b)?).await?))
}
async fn create_grant(
    State(s): State<App>,
    Auth(a): Auth,
    b: std::result::Result<Json<CreateGrantRequest>, RejectionResponse>,
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
    b: std::result::Result<Json<UpdateGrantRequest>, RejectionResponse>,
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
    q: std::result::Result<Query<ExpectedRevision>, simple_server::extract::RejectionResponse>,
) -> Result<StatusCode> {
    s.writer
        .revoke_grant(&a, &identifier(&grant)?, &query(q)?.expected_revision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_grants(
    State(s): State<App>,
    Auth(a): Auth,
    q: std::result::Result<Query<ListGrantsQuery>, simple_server::extract::RejectionResponse>,
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
    q: std::result::Result<Query<Callback>, simple_server::extract::RejectionResponse>,
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
    q: std::result::Result<Query<ListRequest>, simple_server::extract::RejectionResponse>,
) -> Result<Json<AllocationPage>> {
    Ok(Json(s.writer.list_allocations(&a, &query(q)?).await?))
}
async fn policy_constraints(
    State(s): State<App>,
    Auth(a): Auth,
    Path(root): Path<String>,
    b: std::result::Result<Json<PolicyConstraintsRequest>, RejectionResponse>,
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

#[cfg(test)]
mod rejection_contract {
    use super::*;

    #[tokio::test]
    async fn protocol_errors_preserve_metadata_and_exact_json_for_rejections_and_handlers() {
        for code in [ErrorCode::Unauthorized, ErrorCode::RateLimited] {
            for extraction in [false, true] {
                let response = if extraction {
                    simple_server::extract::IntoRejectionResponse::into_rejection_response(
                        ApiError(code.clone()),
                    )
                    .into_response()
                } else {
                    ApiError(code.clone()).into_response()
                };
                assert_eq!(response.status().as_u16(), code.http_status());
                assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert_eq!(response.extensions().get::<ErrorCode>(), Some(&code));
                let request_id = response.headers()["x-request-id"].to_str().unwrap();
                let uuid = uuid::Uuid::parse_str(request_id).unwrap();
                assert_eq!(uuid.get_version_num(), 4);
                let expected = ErrorResponse::new(request_id.parse().unwrap(), code.clone());
                if code == ErrorCode::RateLimited {
                    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
                } else {
                    assert!(!response.headers().contains_key(header::RETRY_AFTER));
                }
                assert_eq!(
                    response.into_body().collect(4096).await.unwrap().as_ref(),
                    serde_json::to_vec(&expected).unwrap()
                );
            }
        }
    }
}

#[cfg(test)]
mod response_header_tests {
    use super::*;

    #[tokio::test]
    async fn response_policy_preserves_success_and_normalizes_early_errors_without_storage() {
        let router = Router::new()
            .route(
                "/ok",
                get(|| async {
                    (
                        [
                            ("cache-control", "public, max-age=60"),
                            ("referrer-policy", "unsafe-url"),
                            ("set-cookie", "a=1"),
                        ],
                        "payload",
                    )
                }),
            )
            .route("/error", get(|| async { StatusCode::BAD_REQUEST }))
            .layer(middleware::from_fn(response_headers));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = simple_server::lifecycle::Shutdown::new();
        let stopped = stop.clone();
        let server = tokio::spawn(async move {
            simple_server::web::serve(listener, router, stopped)
                .await
                .unwrap();
        });
        let client = reqwest::Client::new();
        for (method, path, status) in [
            (reqwest::Method::GET, "/ok", 200),
            (reqwest::Method::HEAD, "/ok", 200),
            (reqwest::Method::GET, "/error", 400),
        ] {
            let response = client
                .request(method.clone(), format!("{base}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["referrer-policy"], "no-referrer");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert!(response.headers().contains_key("x-request-id"));
            if path == "/ok" {
                assert_eq!(response.headers()["set-cookie"], "a=1");
            }
            let body = response.text().await.unwrap();
            if method == reqwest::Method::HEAD {
                assert!(body.is_empty());
            } else if path == "/ok" {
                assert_eq!(body, "payload");
            } else {
                assert!(serde_json::from_str::<serde_json::Value>(&body).is_ok());
            }
        }
        stop.request();
        server.await.unwrap();
    }
}
