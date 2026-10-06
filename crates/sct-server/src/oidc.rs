use crate::config::{Config, OidcConfig};
use sct_core::{
    error::{Error, Result},
    lease::Writer,
};
use sct_protocol::ErrorCode;
use simple_server::oidc::{Client, Config as ClientConfig};

#[derive(Clone)]
pub struct Oidc {
    client: Client,
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
        let secret = std::env::var(&oidc.client_secret_env)
            .map_err(|_| "OIDC client secret environment variable is missing")?;
        if secret.is_empty() {
            return Err("OIDC client secret must not be empty");
        }
        let callback = format!("{}/auth/callback", config.public_url.trim_end_matches('/'));
        let client = Client::discover(ClientConfig {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            client_secret: secret,
            redirect_uri: callback,
            allow_loopback_http: config.development,
        })
        .await
        .map_err(|error| error.message())?;
        Ok(Self {
            client,
            issuer: oidc.issuer.clone(),
        })
    }
    pub async fn begin(&self, writer: &Writer) -> Result<(String, String)> {
        let flow = self.client.begin().await.map_err(denied)?;
        let browser = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        writer
            .save_oidc_flow(&flow.state, &browser, &flow.nonce, &flow.pkce_verifier)
            .await?;
        Ok((flow.url, browser))
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
        let subject = self
            .client
            .finish(code, &nonce, &verifier)
            .await
            .map_err(denied)?;
        writer.oidc_session(&self.issuer, &subject).await
    }
}
