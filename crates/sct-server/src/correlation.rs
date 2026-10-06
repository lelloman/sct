//! Keep ScT's server-generated UUID contract while sharing request scope.
use crate::web::{extract::Request, middleware::Next, response::Response};
use simple_server::correlation::{Correlation, HeaderRequestId, Propagation, current_header_id};

pub(crate) fn request_id() -> String {
    current_header_id()
        .map(|id| {
            id.as_header_value()
                .to_str()
                .expect("server UUID")
                .to_owned()
        })
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string())
}

pub(crate) async fn middleware(request: Request, next: Next) -> Response {
    // Never accept a caller's identifier. Preserve UUID v4's wire representation.
    let id = uuid::Uuid::new_v4().simple().to_string();
    Correlation::default()
        .run_selected(
            request,
            HeaderRequestId::new(id.parse().expect("UUID header")),
            Propagation::default(),
            |request| next.run(request),
        )
        .await
}
