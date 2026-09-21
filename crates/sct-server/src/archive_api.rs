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
        .route(
            "/api/v1/archives/{collection}/discs/{disc}/sets",
            get(disc_sets),
        )
        .route(
            "/api/v1/archives/{collection}/discs/{disc}/copies",
            get(copies).post(register_copy),
        )
        .route(
            "/api/v1/archives/{collection}/copies/{copy}",
            simple_server::axum::routing::patch(update_copy),
        )
        .route(
            "/api/v1/archives/{collection}/copies/{copy}/reports",
            get(reports).post(submit_report),
        )
        .route(
            "/api/v1/archives/{collection}/copies/{copy}/events",
            get(copy_events),
        )
        .route("/api/v1/archives/status", get(service_status))
        .route("/api/v1/archives/limits", get(own_limits))
        .route(
            "/api/v1/admin/principals/{principal}/archive-limits",
            get(account_limits).patch(update_account_limits),
        )
        .route("/api/v1/archives/{collection}/discs/{disc}/iso", get(iso))
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

/// One byte range, represented internally with an exclusive end.
fn iso_range(value: &str, length: u64) -> Option<(u64, u64)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if length == 0 {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        return (suffix > 0).then_some((length.saturating_sub(suffix), length));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        length
    } else {
        end.parse::<u64>().ok()?.saturating_add(1).min(length)
    };
    (start < end && start < length).then_some((start, end))
}
async fn iso(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, disc)): Path<(String, String)>,
    headers: simple_server::axum::http::HeaderMap,
    method: simple_server::axum::http::Method,
) -> Result<simple_server::axum::response::Response> {
    use simple_server::axum::{body::Body, http::StatusCode, response::IntoResponse};
    let engine = app
        .transfer
        .as_ref()
        .ok_or(ApiError(ErrorCode::ArchiveArtifactUnavailable))?;
    let collection = id(&collection)?;
    let disc = id(&disc)?;
    let info = engine
        .archive_image_info(&actor, &collection, &disc)
        .await?;
    let length = info
        .artifact
        .length
        .as_ref()
        .ok_or(ApiError(ErrorCode::IntegrityFailure))?
        .get();
    let etag = format!(
        "\"{}\"",
        info.artifact
            .sha256
            .as_ref()
            .ok_or(ApiError(ErrorCode::IntegrityFailure))?
            .as_str()
    );
    if let Some(value) = headers.get("if-match") {
        let matched = value
            .to_str()
            .ok()
            .is_some_and(|s| s == "*" || s.split(',').any(|tag| tag.trim() == etag));
        if !matched {
            let mut response = ApiError(ErrorCode::RevisionConflict).into_response();
            *response.status_mut() = StatusCode::PRECONDITION_FAILED;
            return Ok(response);
        }
    }
    let mut range = None;
    if method != simple_server::axum::http::Method::HEAD
        && headers
            .get("if-range")
            .is_none_or(|v| v.to_str().ok() == Some(etag.as_str()))
        && let Some(value) = headers.get("range")
    {
        range = value.to_str().ok().and_then(|s| iso_range(s, length));
        if range.is_none() {
            let mut response = ApiError(ErrorCode::InvalidRequest).into_response();
            *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
            response.headers_mut().insert(
                "content-range",
                format!("bytes */{length}").parse().unwrap(),
            );
            return Ok(response);
        }
    }
    let (start, end) = range.unwrap_or((0, length));
    let mut response = if method == simple_server::axum::http::Method::HEAD {
        Body::empty().into_response()
    } else {
        let download = engine
            .download_archive_iso(&actor, &collection, &disc, range)
            .await?;
        let stream = futures_util::stream::try_unfold(download, |mut download| async move {
            download
                .next()
                .await
                .map(|next| next.map(|bytes| (bytes, download)))
                .map_err(std::io::Error::other)
        });
        Body::from_stream(stream).into_response()
    };
    let h = response.headers_mut();
    h.insert("etag", etag.parse().unwrap());
    h.insert("accept-ranges", "bytes".parse().unwrap());
    h.insert(
        "content-type",
        "application/x-iso9660-image".parse().unwrap(),
    );
    h.insert("content-length", (end - start).to_string().parse().unwrap());
    h.insert("cache-control", "private, no-store".parse().unwrap());
    h.insert(
        "content-disposition",
        format!("attachment; filename=\"sct-disc-{}.iso\"", info.disc_number)
            .parse()
            .unwrap(),
    );
    if range.is_some() {
        h.insert(
            "content-range",
            format!("bytes {start}-{}/{length}", end - 1)
                .parse()
                .unwrap(),
        );
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    Ok(response)
}

async fn service_status(
    State(app): State<App>,
    Auth(_actor): Auth,
) -> Result<Json<ArchiveServiceStatus>> {
    Ok(Json(app.transfer.as_ref().map_or(
        ArchiveServiceStatus {
            configured: false,
            bundle_version: None,
            platform: None,
        },
        |e| e.archive_service_status(),
    )))
}
async fn own_limits(
    State(app): State<App>,
    Auth(actor): Auth,
) -> Result<Json<ArchiveAccountLimits>> {
    Ok(Json(
        app.writer
            .archive_account_limits(&actor, &id(actor.principal_id())?)
            .await?,
    ))
}
async fn account_limits(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(principal): Path<String>,
) -> Result<Json<ArchiveAccountLimits>> {
    Ok(Json(
        app.writer
            .archive_account_limits(&actor, &id(&principal)?)
            .await?,
    ))
}
async fn update_account_limits(
    State(app): State<App>,
    Auth(actor): Auth,
    Path(principal): Path<String>,
    request: std::result::Result<Json<UpdateArchiveAccountLimitsRequest>, JsonRejection>,
) -> Result<Json<ArchiveAccountLimits>> {
    Ok(Json(
        app.writer
            .update_archive_account_limits(
                &actor,
                &id(&principal)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}

async fn copies(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, disc)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveCopyPage>> {
    Ok(Json(
        app.writer
            .list_archive_copies(
                &actor,
                &id(&collection)?,
                &id(&disc)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn register_copy(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, disc)): Path<(String, String)>,
    request: std::result::Result<Json<RegisterArchiveCopyRequest>, JsonRejection>,
) -> Result<Json<ArchivePhysicalCopy>> {
    Ok(Json(
        app.writer
            .register_archive_copy(
                &actor,
                &id(&collection)?,
                &id(&disc)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn update_copy(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, copy)): Path<(String, String)>,
    request: std::result::Result<Json<UpdateArchiveCopyRequest>, JsonRejection>,
) -> Result<Json<ArchivePhysicalCopy>> {
    Ok(Json(
        app.writer
            .update_archive_copy(
                &actor,
                &id(&collection)?,
                &id(&copy)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn reports(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, copy)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveReportPage>> {
    Ok(Json(
        app.writer
            .list_archive_reports(
                &actor,
                &id(&collection)?,
                &id(&copy)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn submit_report(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, copy)): Path<(String, String)>,
    request: std::result::Result<Json<SubmitArchiveVerificationRequest>, JsonRejection>,
) -> Result<Json<ArchiveVerificationReport>> {
    Ok(Json(
        app.writer
            .submit_archive_verification(
                &actor,
                &id(&collection)?,
                &id(&copy)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}
async fn copy_events(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, copy)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveCopyEventPage>> {
    Ok(Json(
        app.writer
            .list_archive_copy_events(
                &actor,
                &id(&collection)?,
                &id(&copy)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}

async fn disc_sets(
    State(app): State<App>,
    Auth(actor): Auth,
    Path((collection, disc)): Path<(String, String)>,
    request: std::result::Result<Query<ListRequest>, QueryRejection>,
) -> Result<Json<ArchiveSetPage>> {
    Ok(Json(
        app.writer
            .list_archive_disc_sets(
                &actor,
                &id(&collection)?,
                &id(&disc)?,
                &request.map_err(|_| ApiError(ErrorCode::InvalidRequest))?.0,
            )
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::iso_range;
    #[test]
    fn iso_ranges_are_bounded_and_single() {
        for (value, expected) in [
            ("bytes=0-0", Some((0, 1))),
            ("bytes=4-", Some((4, 10))),
            ("bytes=-3", Some((7, 10))),
            ("bytes=-30", Some((0, 10))),
            ("bytes=1-100", Some((1, 10))),
            ("bytes=10-", None),
            ("bytes=-0", None),
            ("bytes=3-2", None),
            ("bytes=0-1,4-5", None),
            ("items=0-1", None),
            ("bytes=18446744073709551616-", None),
        ] {
            assert_eq!(iso_range(value, 10), expected, "{value}");
        }
        assert_eq!(iso_range("bytes=0-", 0), None);
    }
}
