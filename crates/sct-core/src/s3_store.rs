//! S3 path-style SigV4, single-object PUTs (objects <=4 MiB), unversioned buckets.
use crate::{
    backend::Adapter,
    error::{Error, Result, require},
    payload::*,
};
use futures_util::TryStreamExt;
use hmac::{Hmac, Mac};
use sct_protocol::{AdapterDocumentation, ErrorCode, ObservedHealth, digest};
use serde::Deserialize;
use sha2::Sha256;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_env: String,
    secret_key_env: String,
    #[serde(default)]
    session_token_env: Option<String>,
}
pub struct S3Adapter;
pub struct S3Store {
    settings: Settings,
    client: reqwest::Client,
    access: String,
    secret: String,
    token: Option<String>,
    checked: tokio::sync::OnceCell<()>,
}
fn settings(value: &serde_json::Value) -> Result<Settings> {
    let mut s: Settings = serde_json::from_value(value.clone())?;
    let url = reqwest::Url::parse(&s.endpoint).map_err(|_| Error(ErrorCode::InvalidRequest))?;
    require(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && (url.scheme() == "https"
                || (url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))),
        ErrorCode::InvalidRequest,
    )?;
    require(
        (3..=63).contains(&s.bucket.len())
            && s.bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && !s.region.is_empty()
            && s.region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !s.access_key_env.is_empty()
            && !s.secret_key_env.is_empty(),
        ErrorCode::InvalidRequest,
    )?;
    s.endpoint = url.as_str().trim_end_matches('/').into();
    Ok(s)
}
impl S3Store {
    pub fn from_settings(value: &serde_json::Value) -> Result<Self> {
        let s = settings(value)?;
        let env = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .ok_or(Error(ErrorCode::DestinationUnavailable))
        };
        let access = env(&s.access_key_env)?;
        let secret = env(&s.secret_key_env)?;
        let token = s.session_token_env.as_deref().map(env).transpose()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| Error(ErrorCode::InternalError))?;
        Ok(Self {
            settings: s,
            client,
            access,
            secret,
            token,
            checked: tokio::sync::OnceCell::new(),
        })
    }
    async fn request(
        &self,
        method: reqwest::Method,
        key: Option<&ObjectKey>,
        query: &str,
        body: Vec<u8>,
        conditional: bool,
    ) -> Result<reqwest::Response> {
        self.request_using(&self.client, method, key, query, body, conditional)
            .await
    }
    async fn request_using(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        key: Option<&ObjectKey>,
        query: &str,
        body: Vec<u8>,
        conditional: bool,
    ) -> Result<reqwest::Response> {
        let path = match key {
            Some(k) => format!("/{}/{}", self.settings.bucket, k.path()),
            None => format!("/{}", self.settings.bucket),
        };
        let url = format!(
            "{}{}{}",
            self.settings.endpoint,
            path,
            if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            }
        );
        let parsed = reqwest::Url::parse(&url).map_err(|_| Error(ErrorCode::InvalidRequest))?;
        let host = match parsed.port() {
            Some(p) => format!("{}:{p}", parsed.host_str().unwrap()),
            None => parsed.host_str().unwrap().into(),
        };
        let now = chrono::Utc::now();
        let date = now.format("%Y%m%d").to_string();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let payload = digest(&body);
        let mut headers = BTreeMap::from([
            ("host", host),
            ("x-amz-content-sha256", payload.clone()),
            ("x-amz-date", timestamp.clone()),
        ]);
        if conditional {
            headers.insert("if-none-match", "*".into());
        }
        if let Some(token) = &self.token {
            headers.insert("x-amz-security-token", token.clone());
        }
        let names = headers.keys().copied().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{k}:{}\n", v.trim()))
            .collect::<String>();
        let canonical = format!(
            "{}\n{path}\n{query}\n{canonical_headers}\n{names}\n{payload}",
            method.as_str()
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.settings.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
            digest(canonical.as_bytes())
        );
        let date_key = hmac(format!("AWS4{}", self.secret).as_bytes(), date.as_bytes());
        let region_key = hmac(&date_key, self.settings.region.as_bytes());
        let service_key = hmac(&region_key, b"s3");
        let signing_key = hmac(&service_key, b"aws4_request");
        let signature = hmac(&signing_key, to_sign.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut request=client.request(method,url).header("authorization",format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={names}, Signature={signature}",self.access));
        for (k, v) in headers {
            request = request.header(k, v);
        }
        request
            .body(body)
            .send()
            .await
            .map_err(|_| Error(ErrorCode::DestinationUnavailable))
    }
    async fn bucket(&self) -> Result<()> {
        self.checked
            .get_or_try_init(|| async {
                let r = self
                    .request(reqwest::Method::GET, None, "versioning=", vec![], false)
                    .await?;
                require(r.status().is_success(), ErrorCode::DestinationUnavailable)?;
                let xml = small_body(r).await?;
                #[derive(Deserialize)]
                struct Versioning {
                    #[serde(rename = "Status")]
                    status: Option<String>,
                }
                let v: Versioning = quick_xml::de::from_str(&xml)
                    .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
                require(v.status.is_none(), ErrorCode::UnsupportedOperation)?;
                let r = self
                    .request(reqwest::Method::GET, None, "object-lock=", vec![], false)
                    .await?;
                let status = r.status();
                let xml = small_body(r).await?;
                #[derive(Deserialize)]
                struct ServiceError {
                    #[serde(rename = "Code")]
                    code: String,
                }
                let absent = quick_xml::de::from_str::<ServiceError>(&xml)
                    .ok()
                    .is_some_and(|e| {
                        e.code == "ObjectLockConfigurationNotFoundError"
                            || e.code == "ObjectLockConfigurationNotFound"
                    });
                require(
                    status == reqwest::StatusCode::NOT_FOUND && absent,
                    ErrorCode::UnsupportedOperation,
                )?;
                Ok(())
            })
            .await
            .map(|_| ())
    }
}
fn hmac(key: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut h = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts every key length");
    h.update(bytes);
    h.finalize().into_bytes().to_vec()
}
async fn small_body(response: reqwest::Response) -> Result<String> {
    let mut reader =
        tokio_util::io::StreamReader::new(response.bytes_stream().map_err(std::io::Error::other))
            .take(65537);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error(ErrorCode::DestinationUnavailable))?;
    require(bytes.len() <= 65536, ErrorCode::DestinationUnavailable)?;
    String::from_utf8(bytes).map_err(|_| Error(ErrorCode::DestinationUnavailable))
}
impl Adapter for S3Adapter {
    fn documentation(&self) -> AdapterDocumentation {
        AdapterDocumentation{kind:"s3".into(),display_name:"S3 unversioned storage".into(),acknowledgement:"SigV4 conditional PUT followed by SHA-256 verified GET".into(),consistency:"Requires S3 strong read-after-write/delete semantics".into(),durability:"Provider acknowledgment; no independent durability certification".into(),deletion_and_capacity:"Confirmed zero-byte tombstone PUT and configured catalog capacity budget".into(),limitations:vec!["Unversioned buckets without Object Lock only; do not change these settings while active".into(),"S3-compatible integration qualification is not AWS qualification".into()]}
    }
    fn validate(&self, value: &serde_json::Value) -> Result<String> {
        let s = settings(value)?;
        Ok(format!("s3:{}/{}", s.endpoint, s.bucket))
    }
    fn probe(&self, _: &serde_json::Value) -> ObservedHealth {
        ObservedHealth::Unknown
    }
    fn open(&self, value: &serde_json::Value) -> Result<Arc<dyn ObjectStore>> {
        Ok(Arc::new(S3Store::from_settings(value)?))
    }
}
impl ObjectStore for S3Store {
    fn put<'a>(&'a self, key: &'a ObjectKey, length: u64, body: Reader) -> IoFuture<'a, ()> {
        Box::pin(async move {
            self.bucket().await?;
            let bytes = verified(body, key, length).await?;
            let r = self
                .request(reqwest::Method::PUT, Some(key), "", bytes, true)
                .await?;
            require(
                r.status().is_success() || r.status() == reqwest::StatusCode::PRECONDITION_FAILED,
                ErrorCode::DestinationUnavailable,
            )?;
            verified(self.read(key).await?, key, length).await?;
            Ok(())
        })
    }
    fn read<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Reader> {
        Box::pin(async move {
            self.bucket().await?;
            let r = self
                .request(reqwest::Method::GET, Some(key), "", vec![], false)
                .await?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(Error(ErrorCode::NotFound));
            }
            require(r.status().is_success(), ErrorCode::DestinationUnavailable)?;
            Ok(Box::pin(tokio_util::io::StreamReader::new(
                r.bytes_stream().map_err(std::io::Error::other),
            )) as Reader)
        })
    }
    fn read_with_policy<'a>(
        &'a self,
        key: &'a ObjectKey,
        policy: &'a sct_protocol::ReadCandidate,
    ) -> IoFuture<'a, Reader> {
        Box::pin(async move {
            self.bucket().await?;
            // No fixed body timeout: the caller enforces idle/progress/deadline budgets.
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_millis(policy.connection_timeout_ms.get()))
                .build()
                .map_err(|_| Error(ErrorCode::InternalError))?;
            let response = self
                .request_using(&client, reqwest::Method::GET, Some(key), "", vec![], false)
                .await?;
            require(
                response.status().is_success(),
                ErrorCode::DestinationUnavailable,
            )?;
            Ok(Box::pin(tokio_util::io::StreamReader::new(
                response.bytes_stream().map_err(std::io::Error::other),
            )) as Reader)
        })
    }
    fn stat<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, Option<u64>> {
        Box::pin(async move {
            self.bucket().await?;
            let r = self
                .request(reqwest::Method::HEAD, Some(key), "", vec![], false)
                .await?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            require(r.status().is_success(), ErrorCode::DestinationUnavailable)?;
            Ok(Some(
                r.headers()
                    .get("content-length")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.parse().ok())
                    .ok_or(Error(ErrorCode::DestinationUnavailable))?,
            ))
        })
    }
    fn delete<'a>(&'a self, key: &'a ObjectKey) -> IoFuture<'a, DeleteOutcome> {
        Box::pin(async move {
            self.bucket().await?;
            // An empty immutable-key tombstone is a physical write fence. Unlike
            // DELETE, it prevents an older conditional PUT finishing afterward.
            let r = self
                .request(reqwest::Method::PUT, Some(key), "", vec![], false)
                .await?;
            require(r.status().is_success(), ErrorCode::DestinationUnavailable)?;
            Ok(DeleteOutcome::Confirmed)
        })
    }
    fn available(&self) -> IoFuture<'_, Option<u64>> {
        Box::pin(async {
            self.bucket().await?;
            Ok(None)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires disposable S3-compatible service"]
    async fn real_s3_conformance() {
        let endpoint = std::env::var("SCT_TEST_S3_ENDPOINT").expect("S3 endpoint required");
        let settings = serde_json::json!({"endpoint":endpoint,"bucket":"sct-test","region":"us-east-1","access_key_env":"SCT_TEST_S3_ACCESS","secret_key_env":"SCT_TEST_S3_SECRET"});
        let store = S3Store::from_settings(&settings).unwrap();
        let response = store
            .request(reqwest::Method::PUT, None, "", vec![], false)
            .await
            .unwrap();
        assert!(response.status().is_success() || response.status().as_u16() == 409);
        crate::payload::tests::conformance(&store).await;
        let key = crate::payload::tests::key(b"good", "corrupted");
        store
            .request(reqwest::Method::PUT, Some(&key), "", b"bad".to_vec(), false)
            .await
            .unwrap();
        assert!(
            verified(store.read(&key).await.unwrap(), &key, 4)
                .await
                .is_err()
        );
        store.delete(&key).await.unwrap();
        let mut wrong = S3Store::from_settings(&settings).unwrap();
        wrong.secret = "incorrect-secret".into();
        assert!(wrong.available().await.is_err());
        let mut versioned = settings.clone();
        versioned["bucket"] = "sct-versioned".into();
        let versioned = S3Store::from_settings(&versioned).unwrap();
        assert!(
            versioned
                .request(reqwest::Method::PUT, None, "", vec![], false)
                .await
                .unwrap()
                .status()
                .is_success()
        );
        let xml=br#"<VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Status>Enabled</Status></VersioningConfiguration>"#.to_vec();
        assert!(
            versioned
                .request(reqwest::Method::PUT, None, "versioning=", xml, false)
                .await
                .unwrap()
                .status()
                .is_success()
        );
        assert!(matches!(
            versioned.available().await.unwrap_err().0,
            ErrorCode::UnsupportedOperation
        ));
    }
}
