//! Standalone transport foundation. Storage operations follow their protocol contracts.

use std::time::Duration;
mod transfer;
mod tree;
pub use transfer::UploadJob;
pub use tree::{TreeJob, TreeMutation};

pub use sct_protocol as protocol;

/// A client for a server origin (for example `https://storage.example/`).
/// Cross-origin redirects are disabled, including for future authenticated requests.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    origin: reqwest::Url,
    token: Option<String>,
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
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| "could not initialize HTTP transport")?;
        Ok(Self {
            http,
            origin,
            token: None,
        })
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

#[derive(Debug)]
pub enum Error {
    Transport(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Local(String),
    Integrity,
    SourceChanged,
    Server(Box<protocol::ErrorResponse>),
    UnexpectedStatus(u16),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "local I/O: {e}"),
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::Local(e) => f.write_str(e),
            Self::Integrity => f.write_str("incomplete or invalid transfer"),
            Self::SourceChanged => f.write_str(
                "source changed since planning; capture a stable source and create a new job",
            ),
            Self::Transport(_) => f.write_str("HTTP transport failed"),
            Self::Server(e) => write!(f, "{} (request {})", e.message, e.request_id.as_str()),
            Self::UnexpectedStatus(status) => write!(f, "unexpected HTTP status {status}"),
        }
    }
}
impl std::error::Error for Error {}
impl Client {
    pub fn with_token(mut self, token: String) -> Result<Self, &'static str> {
        if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("invalid application token");
        }
        self.token = Some(token);
        Ok(self)
    }
    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Vec<(&str, String)>,
        body: Option<serde_json::Value>,
    ) -> Result<T, Error> {
        let mut url = self.origin.clone();
        url.set_path(path);
        let mut request = self.http.request(method, url).query(&query);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(Error::Transport)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(match response.json::<protocol::ErrorResponse>().await {
                Ok(error) => Error::Server(Box::new(error)),
                Err(_) => Error::UnexpectedStatus(status),
            });
        }
        response.json().await.map_err(Error::Transport)
    }
    pub async fn capabilities(&self) -> Result<protocol::Capabilities, Error> {
        self.request(reqwest::Method::GET, "/api/v1/capabilities", vec![], None)
            .await
    }
    pub async fn list_roots(
        &self,
        request: &protocol::ListRequest,
    ) -> Result<protocol::RootPage, Error> {
        let mut query = vec![("limit", request.limit.get().to_string())];
        if let Some(cursor) = &request.cursor {
            query.push(("cursor", cursor.as_str().into()));
        }
        self.request(reqwest::Method::GET, "/api/v1/roots", query, None)
            .await
    }
    pub async fn create_root(
        &self,
        request: &protocol::CreateRootRequest,
    ) -> Result<protocol::RootInspection, Error> {
        self.request(
            reqwest::Method::POST,
            "/api/v1/roots",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn inspect_root(
        &self,
        root: &protocol::RootId,
    ) -> Result<protocol::RootInspection, Error> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/roots/{}", root.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn update_root(
        &self,
        root: &protocol::RootId,
        request: &protocol::UpdateRootRequest,
    ) -> Result<protocol::RootInspection, Error> {
        self.request(
            reqwest::Method::PATCH,
            &format!("/api/v1/roots/{}", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn resolve_path(
        &self,
        root: &protocol::RootId,
        path: &protocol::RootPath,
    ) -> Result<protocol::EntryInspection, Error> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/roots/{}/resolve", root.as_str()),
            vec![("path", path.as_str().into())],
            None,
        )
        .await
    }
    pub async fn inspect_entry(
        &self,
        entry: &protocol::EntryId,
    ) -> Result<protocol::EntryInspection, Error> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/entries/{}", entry.as_str()),
            vec![],
            None,
        )
        .await
    }
    pub async fn create_directory(
        &self,
        root: &protocol::RootId,
        request: &protocol::CreateDirectoryRequest,
    ) -> Result<protocol::EntryInspection, Error> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/roots/{}/entries", root.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn list_entries(
        &self,
        root: &protocol::RootId,
        directory: &protocol::EntryId,
        request: &protocol::ListEntriesRequest,
    ) -> Result<protocol::EntryPage, Error> {
        let mut query = vec![
            ("directory_id", directory.as_str().into()),
            ("limit", request.limit.get().to_string()),
            ("include_hidden", request.include_hidden.to_string()),
        ];
        if let Some(cursor) = &request.cursor {
            query.push(("cursor", cursor.token.as_str().into()));
            query.push((
                "directory_revision",
                cursor.directory_revision.as_str().into(),
            ));
        }
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/roots/{}/entries", root.as_str()),
            query,
            None,
        )
        .await
    }
    pub async fn rename_entry(
        &self,
        entry: &protocol::EntryId,
        request: &protocol::RenameEntryRequest,
    ) -> Result<protocol::EntryInspection, Error> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/entries/{}/rename", entry.as_str()),
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
    pub async fn effective_policy(
        &self,
        scope: &protocol::PolicyScope,
    ) -> Result<protocol::EffectivePolicy, Error> {
        let query = match scope {
            protocol::PolicyScope::Root { root_id } => vec![
                ("kind", "root".into()),
                ("root_id", root_id.as_str().into()),
            ],
            protocol::PolicyScope::Entry { root_id, entry_id } => vec![
                ("kind", "entry".into()),
                ("root_id", root_id.as_str().into()),
                ("entry_id", entry_id.as_str().into()),
            ],
        };
        self.request(reqwest::Method::GET, "/api/v1/policies", query, None)
            .await
    }
    pub async fn set_policy(
        &self,
        request: &protocol::SetPolicyRequest,
    ) -> Result<protocol::EffectivePolicy, Error> {
        self.request(
            reqwest::Method::PATCH,
            "/api/v1/policies",
            vec![],
            Some(serde_json::to_value(request).expect("wire DTO")),
        )
        .await
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
