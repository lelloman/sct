use sct_core::backend::{Registry, TestAdapter};
use sct_protocol::{BackendInspection, ByteCount, PoolId};
use serde::Deserialize;
use std::{collections::BTreeSet, net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bind: SocketAddr,
    pub public_url: String,
    pub web_dir: PathBuf,
    #[serde(default = "checkpoint_directory_default")]
    pub checkpoint_dir: PathBuf,
    #[serde(default)]
    pub archive: Option<sct_core::archive_export::ArchiveExportConfig>,
    pub database_url_env: String,
    #[serde(default)]
    pub development: bool,
    #[serde(default)]
    pub enable_test_adapter: bool,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub pools: Vec<PoolConfig>,
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
    #[serde(default = "worker_default")]
    pub workers: u16,
    #[serde(default = "expiry_default")]
    pub session_expiry_seconds: u32,
}
fn worker_default() -> u16 {
    4
}
fn checkpoint_directory_default() -> PathBuf {
    "checkpoints".into()
}
fn expiry_default() -> u32 {
    86400
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret_env: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    pub id: PoolId,
    pub capacity_bytes: ByteCount,
    pub overhead_bytes: ByteCount,
    pub staging_bytes: ByteCount,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    pub inspection: BackendInspection,
    pub pool_id: PoolId,
    pub settings: serde_json::Value,
}
impl Config {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let mut config: Self = toml::from_str(input)
            .map_err(|_| "invalid TOML configuration: check documented fields and value types")?;
        config.validate()?;
        config.public_url = reqwest::Url::parse(&config.public_url)
            .map_err(|_| "invalid public URL")?
            .origin()
            .ascii_serialization();
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_url(&self.public_url, self.development)?;
        if reqwest::Url::parse(&self.public_url)
            .map_err(|_| "invalid public URL")?
            .path()
            != "/"
        {
            return Err("public_url must be an origin without a path prefix");
        }
        if self.workers == 0
            || self.workers > 256
            || !(60..=2592000).contains(&self.session_expiry_seconds)
        {
            return Err("invalid worker count or session expiry");
        }
        if self.enable_test_adapter && !self.development {
            return Err("test adapters require development=true");
        }
        if self.database_url_env.is_empty() {
            return Err("database_url_env is required");
        }
        if let Some(oidc) = &self.oidc {
            validate_url(&oidc.issuer, self.development)?;
            if oidc.client_id.is_empty() || oidc.client_secret_env.is_empty() {
                return Err("OIDC client_id and client_secret_env are required");
            }
        }
        let mut pools = BTreeSet::new();
        for p in &self.pools {
            if !pools.insert(p.id.clone())
                || p.overhead_bytes.get() > p.capacity_bytes.get()
                || p.staging_bytes.get() > p.capacity_bytes.get() - p.overhead_bytes.get()
            {
                return Err("invalid or duplicate capacity pool budget");
            }
        }
        let registry = self.registry()?;
        let mut backends = BTreeSet::new();
        let mut destinations = BTreeSet::new();
        for b in &self.backends {
            let identity = registry
                .validate(&b.inspection, &b.settings)
                .map_err(|_| "invalid backend kind or settings")?;
            if !pools.contains(&b.pool_id)
                || !backends.insert(b.inspection.id.clone())
                || !destinations.insert(identity)
            {
                return Err("unknown pool, duplicate backend or aliased destination");
            }
        }
        Ok(())
    }
    pub fn registry(&self) -> Result<Registry, &'static str> {
        let mut registry = Registry::default();
        registry
            .register(Arc::new(sct_core::local_store::LocalAdapter))
            .map_err(|_| "duplicate adapter")?;
        registry
            .register(Arc::new(sct_core::s3_store::S3Adapter))
            .map_err(|_| "duplicate adapter")?;
        if self.enable_test_adapter {
            registry
                .register(Arc::new(TestAdapter))
                .map_err(|_| "duplicate adapter")?;
        }
        Ok(registry)
    }
    /// Environment overrides are explicit and limited; secrets are never Debug printed.
    pub fn load(path: &std::path::Path) -> Result<Self, &'static str> {
        let input = std::fs::read_to_string(path).map_err(|_| "cannot read SCT_CONFIG")?;
        let mut config = Self::parse(&input)?;
        if let Some(bind) = override_value("SCT_BIND")? {
            config.bind = bind.parse().map_err(|_| "invalid SCT_BIND")?;
        }
        if let Some(path) = override_value("SCT_WEB_DIR")? {
            config.web_dir = path.into();
        }
        if let Some(url) = override_value("SCT_PUBLIC_URL")? {
            config.public_url = url;
        }
        config.validate()?;
        config.public_url = reqwest::Url::parse(&config.public_url)
            .map_err(|_| "invalid public URL")?
            .origin()
            .ascii_serialization();
        Ok(config)
    }
}
fn validate_url(value: &str, development: bool) -> Result<(), &'static str> {
    let url = reqwest::Url::parse(value).map_err(|_| "invalid public or issuer URL")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(development && local && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "URLs require HTTPS (loopback HTTP only in development), without credentials, query or fragment",
        );
    }
    Ok(())
}

fn override_value(name: &str) -> Result<Option<String>, &'static str> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err("configuration environment overrides must be valid UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASIC: &str = "bind='127.0.0.1:8080'\npublic_url='https://storage.example'\nweb_dir='web/dist'\ndatabase_url_env='SCT_DATABASE_URL'\n";
    #[test]
    fn rejects_unknown_fields_and_unsafe_configuration_without_echoing_secrets() {
        assert!(Config::parse(BASIC).is_ok());
        for extra in [
            "unknown='secret'",
            "workers=0",
            "enable_test_adapter=true",
            "session_expiry_seconds=0",
        ] {
            let err = Config::parse(&format!("{BASIC}{extra}")).err().unwrap();
            assert!(!err.contains("secret"));
        }
        assert!(
            Config::parse(&BASIC.replace("https://storage.example", "http://storage.example"))
                .is_err()
        );
        assert!(
            Config::parse(&BASIC.replace(
                "https://storage.example",
                "https://user:password@storage.example"
            ))
            .is_err()
        );
    }
}
