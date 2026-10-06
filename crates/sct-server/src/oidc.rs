use crate::config::{Config, OidcConfig};
use openidconnect::{
    AccessTokenHash, AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointMaybeSet, EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
};
use sct_core::{
    error::{Error, Result},
    lease::Writer,
};
use sct_protocol::ErrorCode;

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
#[derive(Clone)]
pub struct Oidc {
    client: Client,
    http: OidcHttp,
    issuer: String,
}
fn denied<T>(_: T) -> Error {
    Error(ErrorCode::Unauthorized)
}
impl Oidc {
    pub async fn discover(
        config: &Config,
        oidc: &OidcConfig,
    ) -> std::result::Result<Self, &'static str> {
        let http = simple_server::client::Client::builder()
            .https_only(!config.development)
            .no_redirect()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| "cannot initialize OIDC HTTP client")?;
        let http = OidcHttp(http);
        let issuer = IssuerUrl::new(oidc.issuer.clone()).map_err(|_| "invalid OIDC issuer")?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|_| "OIDC discovery failed")?;
        // Discovery is trusted configuration, but never permit HTTPS downgrades.
        for url in [
            Some(metadata.authorization_endpoint().url()),
            metadata.token_endpoint().map(|u| u.url()),
            Some(metadata.jwks_uri().url()),
        ]
        .into_iter()
        .flatten()
        {
            if url.scheme() != "https"
                && !(config.development
                    && url.scheme() == "http"
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
            {
                return Err("unsafe OIDC endpoint");
            }
        }
        let secret = std::env::var(&oidc.client_secret_env)
            .map_err(|_| "OIDC client secret environment variable is missing")?;
        if secret.is_empty() {
            return Err("OIDC client secret must not be empty");
        }
        let callback = format!("{}/auth/callback", config.public_url.trim_end_matches('/'));
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(oidc.client_id.clone()),
            Some(ClientSecret::new(secret)),
        )
        .set_redirect_uri(RedirectUrl::new(callback).map_err(|_| "invalid OIDC callback URL")?);
        Ok(Self {
            client,
            http,
            issuer: oidc.issuer.clone(),
        })
    }
    pub async fn begin(&self, writer: &Writer) -> Result<(String, String)> {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = self
            .client
            .authorize_url(
                AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge)
            .url();
        let browser = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        writer
            .save_oidc_flow(state.secret(), &browser, nonce.secret(), verifier.secret())
            .await?;
        Ok((url.into(), browser))
    }
    pub async fn finish(
        &self,
        writer: &Writer,
        state: &str,
        browser: &str,
        code: &str,
    ) -> Result<sct_core::auth::IssuedCredential> {
        if state.len() > 512 || browser.len() != 64 || code.len() > 4096 {
            return Err(Error(ErrorCode::Unauthorized));
        }
        let (nonce, verifier) = writer.consume_oidc_flow(state, browser).await?;
        let response = self
            .client
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(denied)?
            .set_pkce_verifier(PkceCodeVerifier::new(verifier))
            .request_async(&self.http)
            .await
            .map_err(denied)?;
        let token = response.id_token().ok_or(Error(ErrorCode::Unauthorized))?;
        let verifier = self.client.id_token_verifier();
        let claims = token
            .claims(&verifier, &Nonce::new(nonce))
            .map_err(denied)?;
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                response.access_token(),
                token.signing_alg().map_err(denied)?,
                token.signing_key(&verifier).map_err(denied)?,
            )
            .map_err(denied)?;
            if actual != *expected {
                return Err(Error(ErrorCode::Unauthorized));
            }
        }
        writer
            .oidc_session(&self.issuer, claims.subject().as_str())
            .await
    }
}

#[derive(Clone)]
struct OidcHttp(simple_server::client::Client);
impl<'a> openidconnect::AsyncHttpClient<'a> for OidcHttp {
    type Error = std::io::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = ResultHttp> + Send + 'a>>;
    fn call(&'a self, request: openidconnect::HttpRequest) -> Self::Future {
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let response = self
                .0
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(std::io::Error::other)?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response
                .bytes()
                .await
                .map_err(std::io::Error::other)?
                .to_vec();
            let mut result = openidconnect::HttpResponse::new(body);
            *result.status_mut() = status;
            *result.headers_mut() = headers;
            Ok(result)
        })
    }
}
type ResultHttp = std::result::Result<openidconnect::HttpResponse, std::io::Error>;
