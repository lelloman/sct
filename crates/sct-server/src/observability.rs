//! Bounded route labels and structured metadata-only request logs.
use crate::api::{ApiError, App, Auth};
use simple_server::axum::{
    Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

const BUCKETS: [f64; 6] = [0.005, 0.025, 0.1, 0.5, 2.0, 10.0];
#[derive(Default)]
struct ResponseStats {
    count: u64,
    seconds: f64,
    buckets: [u64; 6],
}
#[derive(Default)]
pub(crate) struct HttpMetrics {
    responses: Mutex<BTreeMap<(String, &'static str, u16), ResponseStats>>,
    active: AtomicU64,
}
struct Active(Arc<HttpMetrics>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}
fn method(value: &str) -> &'static str {
    match value {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    }
}
pub(crate) async fn observe(
    State(metrics): State<Arc<HttpMetrics>>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    // MatchedPath is a router template, never a supplied URL, query or filename.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let method = method(request.method().as_str());
    metrics.active.fetch_add(1, Ordering::Relaxed);
    let _active = Active(metrics.clone());
    let mut response = next.run(request).await;
    let request_id = crate::correlation::request_id();
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("server correlation ID"),
    );
    let elapsed = started.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    if let Ok(mut values) = metrics.responses.lock() {
        let record = values
            .entry((route.clone(), method, status / 100))
            .or_default();
        record.count += 1;
        record.seconds += elapsed;
        for (i, limit) in BUCKETS.iter().enumerate() {
            if elapsed <= *limit {
                record.buckets[i] += 1;
            }
        }
    }
    eprintln!(
        "{}",
        serde_json::json!({"event":"http_response","request_id":request_id,"method":method,"route":route,"status":status,"duration_ms":(elapsed*1000.0) as u64,"code":response.extensions().get::<sct_protocol::ErrorCode>()})
    );
    response
}
fn group(out: &mut String, name: &str, labels: &[&str], values: &[(String, i64)]) {
    out.push_str(&format!("# TYPE {name} gauge\n"));
    for label in labels {
        let count: i64 = values
            .iter()
            .filter(|(value, _)| value == label)
            .map(|(_, count)| count)
            .sum();
        out.push_str(&format!("{name}{{state=\"{label}\"}} {count}\n"));
    }
}
impl HttpMetrics {
    fn render(&self) -> String {
        let mut out = format!(
            "# TYPE sct_http_active_requests gauge\nsct_http_active_requests {}\n# TYPE sct_http_responses_total counter\n# TYPE sct_http_response_seconds histogram\n",
            self.active.load(Ordering::Relaxed)
        );
        if let Ok(values) = self.responses.lock() {
            for ((route, method, class), record) in values.iter() {
                let labels =
                    format!("route=\"{route}\",method=\"{method}\",status_class=\"{class}xx\"");
                out.push_str(&format!(
                    "sct_http_responses_total{{{labels}}} {}\n",
                    record.count
                ));
                for (i, limit) in BUCKETS.iter().enumerate() {
                    out.push_str(&format!(
                        "sct_http_response_seconds_bucket{{{labels},le=\"{limit}\"}} {}\n",
                        record.buckets[i]
                    ));
                }
                out.push_str(&format!("sct_http_response_seconds_bucket{{{labels},le=\"+Inf\"}} {}\nsct_http_response_seconds_count{{{labels}}} {}\nsct_http_response_seconds_sum{{{labels}}} {}\n",record.count,record.count,record.seconds));
            }
        }
        out
    }
}
pub(crate) fn install(router: Router, app: App) -> Router {
    let metrics = Arc::new(HttpMetrics::default());
    let scrape = metrics.clone();
    let endpoint = Router::new().route("/metrics",get(move |State(app):State<App>,Auth(actor):Auth| {
        let metrics = scrape.clone();
        async move {
            let snapshot = tokio::time::timeout(std::time::Duration::from_secs(5),app.writer.service_metrics(&actor)).await
                .map_err(|_|ApiError(sct_protocol::ErrorCode::DestinationUnavailable))??;
            let mut out = metrics.render();
            group(&mut out,"sct_roots", &["enabled","disabled","recovering"], &snapshot.root_states);
            group(&mut out,"sct_operations", &["active","recovering","cleaning_up","completed","aborted","failed"], &snapshot.operation_states);
            group(&mut out,"sct_maintenance_jobs", &["queued","running","retry_wait","blocked","completed","failed"], &snapshot.maintenance_states);
            group(&mut out,"sct_checkpoints", &["fencing","exporting","exported","failed","released"], &snapshot.checkpoint_states);
            group(&mut out,"sct_backup_freshness", &["not_configured","never_committed","on_time","overdue"], &snapshot.freshness);
            group(&mut out,"sct_backend_health", &["unknown","reachable","degraded","unavailable"], &snapshot.backend_health);
            for (kind,value) in snapshot.capacity { out.push_str(&format!("# TYPE sct_allocation_{kind}_bytes gauge\nsct_allocation_{kind}_bytes {value}\n")); }
            Ok::<_,ApiError>((StatusCode::OK,[("content-type","text/plain; version=0.0.4; charset=utf-8"),("cache-control","no-store")],out).into_response())
        }
    })).with_state(app);
    router
        .merge(endpoint)
        .layer(middleware::from_fn_with_state(metrics, observe))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregates_use_fixed_labels_and_cumulative_histograms() {
        let metrics = HttpMetrics::default();
        metrics.responses.lock().unwrap().insert(
            ("/api/v1/roots/{root}".into(), "GET", 2),
            ResponseStats {
                count: 2,
                seconds: 0.6,
                buckets: [0, 0, 1, 2, 2, 2],
            },
        );
        let mut rendered = metrics.render();
        group(
            &mut rendered,
            "sct_backend_health",
            &["unknown", "reachable"],
            &[("private-root-id".into(), 1), ("reachable".into(), 3)],
        );
        assert!(rendered.contains("status_class=\"2xx\",le=\"+Inf\"} 2"));
        assert!(rendered.contains("sct_backend_health{state=\"reachable\"} 3"));
        assert!(!rendered.contains("private-root-id"));
        assert_eq!(method("UNTRUSTED"), "OTHER");
    }
}
