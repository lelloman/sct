//! Standalone transport foundation. Storage operations follow their protocol contracts.

use std::time::Duration;

pub use sct_protocol as protocol;

/// A client for a server origin (for example `https://storage.example/`).
/// Cross-origin redirects are disabled, including for future authenticated requests.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    origin: reqwest::Url,
}

impl Client {
    pub fn new(origin: reqwest::Url) -> Result<Self, &'static str> {
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err("expected an HTTP(S) origin without credentials, path, query or fragment");
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "could not initialize HTTP transport")?;
        Ok(Self { http, origin })
    }

    /// Check process liveness; this does not imply storage readiness.
    pub async fn check_liveness(&self) -> Result<(), reqwest::Error> {
        let mut url = self.origin.clone();
        url.set_path("/health/live");
        self.http.get(url).send().await?.error_for_status()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_unambiguous_server_origins() {
        for origin in ["http://localhost:8080", "https://storage.example/"] {
            assert!(Client::new(origin.parse().unwrap()).is_ok());
        }
        for origin in [
            "ftp://storage.example/",
            "https://user:secret@storage.example/",
            "https://storage.example/nested/",
            "https://storage.example/?query=value",
            "https://storage.example/#fragment",
        ] {
            assert!(Client::new(origin.parse().unwrap()).is_err());
        }
    }
}
