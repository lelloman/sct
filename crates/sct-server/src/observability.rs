//! Bounded route labels and structured metadata-only request logs.
use crate::api::{ApiError, App, Auth};
use crate::web::{
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
    time::Duration,
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
struct HttpObserver {
    metrics: Arc<HttpMetrics>,
    route: String,
    method: &'static str,
    request_id: String,
}
impl HttpObserver {
    fn response_event(
        &self,
        response: &crate::web::tracing::ResponseInfo<'_>,
        latency: Duration,
    ) -> serde_json::Value {
        serde_json::json!({"event":"http_response","request_id":self.request_id,"method":self.method,"route":self.route,"status":response.status().as_u16(),"duration_ms":latency.as_millis() as u64,"code":response.extensions().get::<sct_protocol::ErrorCode>()})
    }
}
impl crate::web::tracing::Observer for HttpObserver {
    fn on_response(
        &mut self,
        _span: &tracing::Span,
        response: &crate::web::tracing::ResponseInfo<'_>,
        latency: Duration,
    ) {
        let elapsed = latency.as_secs_f64();
        if let Ok(mut values) = self.metrics.responses.lock() {
            let record = values
                .entry((
                    self.route.clone(),
                    self.method,
                    response.status().as_u16() / 100,
                ))
                .or_default();
            record.count += 1;
            record.seconds += elapsed;
            for (i, limit) in BUCKETS.iter().enumerate() {
                if elapsed <= *limit {
                    record.buckets[i] += 1;
                }
            }
        }
        // Preserve ScT's unconditional metadata-only stderr contract. This
        // observer replaces default events; it requires no global subscriber.
        eprintln!("{}", self.response_event(response, latency));
    }
}
pub(crate) async fn observe(
    State(metrics): State<Arc<HttpMetrics>>,
    request: Request,
    next: Next,
) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let method = method(request.method().as_str());
    let request_id = crate::correlation::request_id();
    metrics.active.fetch_add(1, Ordering::Relaxed);
    // The existing active metric ends at headers, not after body streaming.
    let _active = Active(metrics.clone());
    let observer = HttpObserver {
        metrics,
        route,
        method,
        request_id: request_id.clone(),
    };
    crate::web::tracing::trace_with_observer(request, observer, |request| async move {
        let mut response = next.run(request).await;
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&request_id).expect("server correlation ID"),
        );
        response
    })
    .await
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
            let snapshot = simple_server::time::timeout(std::time::Duration::from_secs(5),app.writer.service_metrics(&actor)).await
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
fn sct_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(
        simple_server::runtime::Runtime::try_current()
            .expect("test engine")
            .scope(future),
    )
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
    #[test]
    fn owned_event_preserves_approved_metadata_and_header_latency() {
        let observer = HttpObserver {
            metrics: Arc::new(HttpMetrics::default()),
            route: "/api/v1/roots/{root}".into(),
            method: "GET",
            request_id: "0123456789abcdef0123456789abcdef".into(),
        };
        let mut response = StatusCode::NOT_FOUND.into_response();
        response
            .extensions_mut()
            .insert(sct_protocol::ErrorCode::UnsupportedOperation);
        assert_eq!(
            observer.response_event(
                &crate::web::tracing::ResponseInfo::new(&response),
                Duration::from_micros(12_999),
            ),
            serde_json::json!({"event":"http_response","request_id":"0123456789abcdef0123456789abcdef","method":"GET","route":"/api/v1/roots/{root}","status":404,"duration_ms":12,"code":sct_protocol::ErrorCode::UnsupportedOperation})
        );
    }

    #[simple_server::test(host_runtime = true)]
    async fn shared_observer_records_headers_without_subscriber_or_body_completion() {
        use crate::web::body::Body;
        let metrics = Arc::new(HttpMetrics::default());
        let app = Router::new()
            .route(
                "/stream/{id}",
                get(|| async {
                    let first = futures_util::stream::once(async {
                        Ok::<_, std::io::Error>(crate::web::body::Bytes::from_static(b"first"))
                    });
                    let pending = futures_util::stream::pending();
                    use futures_util::StreamExt;
                    Body::from_stream(first.chain(pending))
                }),
            )
            .layer(middleware::from_fn_with_state(metrics.clone(), observe))
            .layer(middleware::from_fn(crate::correlation::middleware));
        let listener = crate::web::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = super::sct_spawn(async move {
            crate::web::serve(
                listener,
                app,
                simple_server::engine_lifecycle::Shutdown::new(),
            )
            .await
            .unwrap();
        });
        let http = reqwest::Client::new();
        let response = http
            .get(format!(
                "http://{address}/stream/private-id?secret=private-secret"
            ))
            .header("x-request-id", "caller-secret")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"].to_str().unwrap();
        assert_eq!(id.len(), 32);
        assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 4);
        assert_eq!(metrics.active.load(Ordering::Relaxed), 0);
        let rendered = metrics.render();
        assert!(rendered.contains(
            "sct_http_responses_total{route=\"/stream/{id}\",method=\"GET\",status_class=\"2xx\"} 1"
        ));
        assert!(!rendered.contains("private"));
        // No EOF has occurred: headers already updated the existing histogram.
        assert_eq!(metrics.responses.lock().unwrap().len(), 1);
        drop(response);
        let response = http
            .post(format!("http://{address}/stream/private-id"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(metrics.active.load(Ordering::Relaxed), 0);
        assert!(
            metrics
                .render()
                .contains("method=\"POST\",status_class=\"4xx\"} 1")
        );
        server.abort();
    }
}
