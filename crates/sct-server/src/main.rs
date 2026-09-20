use simple_server::http;
use simple_server::lifecycle::{Lifecycle, Shutdown, ShutdownOptions, Signals};
use std::time::Duration;

use sct_core::{
    catalog::Catalog,
    roots::{BackendSetup, PoolSetup},
};
use sct_server::{api::App, config::Config};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = match std::env::var("SCT_CONFIG") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("SCT_CONFIG must be valid UTF-8".into()),
    };
    if let Some(path) = config_path {
        let config = Config::load(std::path::Path::new(&path))?;
        let url = std::env::var(&config.database_url_env)
            .map_err(|_| "database secret environment variable missing")?;
        let catalog = Catalog::new(&url).map_err(|_| "invalid PostgreSQL configuration")?;
        catalog.migrate().await.map_err(
            |_| "catalog migration failed; check database connectivity and schema version",
        )?;
        let writer = catalog
            .acquire_writer()
            .await
            .map_err(|_| "cannot acquire writer lease; another server may be active")?;
        if !args.is_empty() {
            let result = local_command(&writer, &args).await;
            writer.release().await?;
            catalog.close().await;
            return result;
        }
        require_web(&config.web_dir)?;
        let registry = config.registry()?;
        let pools: Vec<_> = config
            .pools
            .iter()
            .map(|p| PoolSetup {
                id: p.id.clone(),
                capacity: (p.capacity_bytes.get() - p.overhead_bytes.get())
                    .try_into()
                    .expect("validated budget"),
                staging: p.staging_bytes,
            })
            .collect();
        let backends: Vec<_> = config
            .backends
            .iter()
            .map(|b| BackendSetup {
                pool_id: b.pool_id.clone(),
                inspection: b.inspection.clone(),
                settings: b.settings.clone(),
            })
            .collect();
        writer.configure(&registry,&pools,&backends).await.map_err(|_|"infrastructure configuration conflicts with catalog allocations or destinations")?;
        let oidc = match &config.oidc {
            Some(oidc) => Some(Arc::new(
                sct_server::oidc::Oidc::discover(&config, oidc).await?,
            )),
            None => None,
        };
        let mut stores = std::collections::BTreeMap::new();
        for backend in writer.configured_backends().await? {
            if backend.inspection.adapter_kind != "test" {
                stores.insert(
                    backend.inspection.id.as_str().to_owned(),
                    registry
                        .get(&backend.inspection.adapter_kind)?
                        .open(&backend.settings)?,
                );
            }
        }
        let transfer = sct_core::transfer_io::TransferEngine::new(
            writer.clone(),
            stores,
            config.workers,
            config.session_expiry_seconds,
        )?
        .with_checkpoint_directory(config.checkpoint_dir.clone());
        let worker = transfer.clone();
        let recovery = tokio::spawn(async move {
            loop {
                if worker.tick().await.is_err() {
                    eprintln!("transfer recovery will retry");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
        let heartbeat = transfer.clone();
        let app = App {
            transfer: Some(transfer),
            writer: writer.clone(),
            registry,
            oidc,
            public_url: config.public_url.clone(),
            secure_cookies: config.public_url.starts_with("https://"),
        };
        let signals = Signals::install()?;
        let mut lifecycle = Lifecycle::new(ShutdownOptions {
            grace_period: Duration::from_secs(30),
        });
        let shutdown = lifecycle.shutdown();
        let listener = http::bind(config.bind).await?;
        println!("ScT listening on {}", listener.local_addr()?);
        // Keep ownership alive while HTTP drains; stop renewing only after it finishes.
        let lease_stop = Shutdown::new();
        let stop_after_http = lease_stop.clone();
        lifecycle.service("http", async move {
            let result = http::serve(
                listener,
                sct_server::application(app, config.web_dir),
                shutdown,
            )
            .await;
            stop_after_http.request();
            result
        })?;
        let lease = writer.clone();
        lifecycle.service("writer-lease", async move {
            loop {
                tokio::select! {
                    biased;
                    _ = lease_stop.requested() => return Ok::<_, sct_core::error::Error>(()),
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {
                        lease.renew().await?;
                        heartbeat.heartbeat().await?;
                    }
                }
            }
        })?;
        lifecycle
            .run(signals.wait(), async {
                recovery.abort();
                let _ = recovery.await;
                let released = writer.release().await;
                catalog.close().await;
                released
            })
            .await?;
        println!("ScT graceful shutdown complete");
        return Ok(());
    }
    if !args.is_empty() {
        return Err("local administration requires SCT_CONFIG".into());
    }
    // Retain the explicit development scaffold for frontend-only checks.
    if std::env::var_os("SCT_DATABASE_URL").is_some() {
        return Err("database-backed service requires SCT_CONFIG".into());
    }
    let bind: SocketAddr = std::env::var("SCT_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .map_err(|_| "invalid SCT_BIND")?;
    let web = PathBuf::from(std::env::var("SCT_WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    require_web(&web)?;
    let signals = Signals::install()?;
    let mut lifecycle = Lifecycle::new(ShutdownOptions {
        grace_period: Duration::from_secs(30),
    });
    let listener = http::bind(bind).await?;
    println!(
        "ScT development scaffold listening on {} (set SCT_CONFIG for foundation service)",
        listener.local_addr()?
    );
    lifecycle.service(
        "http",
        http::serve(
            listener,
            sct_server::router(None, web),
            lifecycle.shutdown(),
        ),
    )?;
    lifecycle
        .run(signals.wait(), async { Ok::<_, std::io::Error>(()) })
        .await?;
    println!("ScT graceful shutdown complete");
    Ok(())
}
async fn local_command(
    writer: &sct_core::lease::Writer,
    args: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args == ["recover-admin"] || args == ["bootstrap-admin"] {
        let credential = writer.recover_administrator().await?;
        // This is the one intentional secret output, on the administrator's local terminal.
        println!(
            "{}",
            serde_json::json!({"id":credential.id,"secret":credential.secret})
        );
        return Ok(());
    }
    let secret = std::env::var("SCT_ADMIN_TOKEN")
        .map_err(|_| "SCT_ADMIN_TOKEN required for local principal/credential commands")?;
    let actor = writer.authenticate(&secret, false, None).await?;
    match args {
        [command,principal] if command=="create-principal"=>writer.create_principal(&actor,&principal.parse()?).await?,
        [command,principal] if command=="issue-token"=>{let token=writer.issue_token(&actor,&principal.parse()?).await?;println!("{}",serde_json::json!({"id":token.id,"secret":token.secret}));},
        [command,credential] if command=="revoke-token"=>writer.revoke_credential(&actor,credential).await?,
        _=>return Err("usage: sct-server [bootstrap-admin|recover-admin|create-principal ID|issue-token PRINCIPAL|revoke-token ID]".into())
    }
    Ok(())
}
fn require_web(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !path.join("index.html").is_file() {
        return Err("web build missing; run npm --prefix web run build or set SCT_WEB_DIR".into());
    }
    Ok(())
}
