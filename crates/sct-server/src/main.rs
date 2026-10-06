mod background_worker;

use sct_server::web as http;
use simple_server::engine_lifecycle::{Lifecycle, Shutdown, ShutdownOptions, Signals};
use std::time::Duration;

use sct_core::{
    catalog::Catalog,
    roots::{BackendSetup, PoolSetup},
};
use sct_server::{api::App, config::Config};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[simple_server::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!(
            "{}",
            serde_json::json!({"event":"fatal","message":error.to_string()})
        );
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Recovery has an explicit destination and never loads production config.
    match args.as_slice() {
        [command,package,sources,destination,ack] if command=="copy-checkpoint-payloads" && ack=="--acknowledge-independent" => {
            copy_checkpoint(package,sources,destination).await?;
            return Ok(());
        }
        [command,package,payloads] if command=="verify-checkpoint" => {
            let descriptor=sct_core::recovery::verify_checkpoint_package(std::path::Path::new(package),std::path::Path::new(payloads)).await?;
            println!("{}",serde_json::to_string(&descriptor)?);
            return Ok(());
        }
        [command,package,payloads,destination,option,variable,ack] if command=="recover-checkpoint" && option=="--database-url-env" && ack=="--acknowledge-isolated" => {
            let url=std::env::var(variable).map_err(|_|"recovery database environment variable missing")?;
            let catalog=Catalog::new(&url).map_err(|_|"invalid recovery database configuration")?;
            let descriptor=catalog.recover_checkpoint(std::path::Path::new(package),std::path::Path::new(payloads),std::path::Path::new(destination),true).await?;
            catalog.close().await;
            println!("{}",serde_json::to_string(&descriptor)?);
            return Ok(());
        }
        [command,..] if command=="copy-checkpoint-payloads" || command=="verify-checkpoint" || command=="recover-checkpoint" => return Err("usage: copy-checkpoint-payloads PACKAGE SOURCE_MAP EMPTY_DEST --acknowledge-independent | verify-checkpoint PACKAGE PAYLOADS | recover-checkpoint PACKAGE PAYLOADS EMPTY_DEST --database-url-env VARIABLE --acknowledge-isolated".into()),
        _=>{}
    }
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
        let transfer = match config.archive.clone() {
            Some(archive) => transfer.with_archive_exports(archive).await?,
            None => transfer,
        };
        let archive_worker = transfer.clone();
        let mut archive_exports =
            background_worker::BackgroundWorker::spawn("archive-exports", async move {
                simple_server::task_drivers::run_poll_worker(
                    || true,
                    worker_cadence(),
                    || async {
                        match archive_worker.archive_tick().await {
                            Ok(_) => simple_server::task_drivers::PollOutcome::Worked,
                            Err(error) => {
                                eprintln!(
                                    "{}",
                                    serde_json::json!({"event":"archive_retry","code":error.0})
                                );
                                simple_server::task_drivers::PollOutcome::Failed
                            }
                        }
                    },
                    |_, delay| simple_server::time::sleep(delay),
                )
                .await;
            });
        let worker = transfer.clone();
        let mut recovery = background_worker::BackgroundWorker::spawn("recovery", async move {
            simple_server::task_drivers::run_poll_worker(
                || true,
                worker_cadence(),
                || async {
                    match worker.tick().await {
                        Ok(_) => simple_server::task_drivers::PollOutcome::Worked,
                        Err(error) => {
                            eprintln!(
                                "{}",
                                serde_json::json!({"event":"recovery_retry","code":error.0})
                            );
                            simple_server::task_drivers::PollOutcome::Failed
                        }
                    }
                },
                |_, delay| simple_server::time::sleep(delay),
            )
            .await;
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
        println!(
            "{}",
            serde_json::json!({"event":"server_listening","address":listener.local_addr()?.to_string()})
        );
        // Keep ownership alive while HTTP drains; stop renewing only after it finishes.
        let lease_stop = Shutdown::new();
        let stop_after_http = lease_stop.clone();
        lifecycle.service("http", async move {
            let result = sct_server::web::serve(
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
                match futures_util::future::select(Box::pin(lease_stop.requested()), Box::pin(simple_server::time::sleep(Duration::from_secs(10)))).await {
                    futures_util::future::Either::Left(_) => return Ok::<_, sct_core::error::Error>(()),
                    futures_util::future::Either::Right(_) => {
                        if let Err(error) = lease.renew().await {
                            eprintln!("{}", serde_json::json!({"event":"writer_lease_lost","code":error.0}));
                            return Err(error);
                        }
                        if let Err(error) = heartbeat.heartbeat().await {
                            eprintln!("{}", serde_json::json!({"event":"worker_heartbeat_failed","code":error.0}));
                            return Err(error);
                        }
                    }
                }
            }
        })?;
        lifecycle
            .run(signals.wait(), async {
                archive_exports.stop().await;
                recovery.stop().await;
                let released = writer.release().await;
                catalog.close().await;
                released
            })
            .await?;
        println!("{}", serde_json::json!({"event":"shutdown_complete"}));
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
        sct_server::web::serve(
            listener,
            sct_server::router(None, web),
            lifecycle.shutdown(),
        ),
    )?;
    lifecycle
        .run(signals.wait(), async { Ok::<_, std::io::Error>(()) })
        .await?;
    println!("{}", serde_json::json!({"event":"shutdown_complete"}));
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CopySource {
    kind: String,
    settings: serde_json::Value,
}
async fn copy_checkpoint(
    package: &str,
    sources: &str,
    destination: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use simple_server::io::AsyncReadExt;
    let mut bytes = Vec::new();
    simple_server::fs::File::open(sources)
        .await?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 1024 * 1024 {
        return Err("source map is too large".into());
    }
    let sources: std::collections::BTreeMap<String, CopySource> =
        serde_json::from_slice(&bytes).map_err(|_| "invalid source map")?;
    let destination = std::fs::canonicalize(destination)?;
    let mut registry = sct_core::backend::Registry::default();
    registry.register(Arc::new(sct_core::local_store::LocalAdapter))?;
    registry.register(Arc::new(sct_core::s3_store::S3Adapter))?;
    let mut stores = std::collections::BTreeMap::new();
    for (backend, source) in sources {
        let _: sct_protocol::BackendId = backend.parse()?;
        let adapter = registry.get(&source.kind)?;
        let identity = adapter.validate(&source.settings)?;
        if let Some(path) = identity.strip_prefix("local:") {
            let path = std::path::Path::new(path);
            if destination.starts_with(path) || path.starts_with(&destination) {
                return Err("payload destination must be independent of source storage".into());
            }
        }
        stores.insert(backend, adapter.open(&source.settings)?);
    }
    let descriptor = sct_core::recovery::copy_checkpoint_payloads(
        std::path::Path::new(package),
        &stores,
        &destination,
    )
    .await?;
    println!("{}", serde_json::to_string(&descriptor)?);
    Ok(())
}

// Fixed delay after each completed database-backed cycle, including failures.
// Cancellation remains owned by BackgroundWorker's explicit abort/join sequence.
fn worker_cadence() -> simple_server::task_drivers::PollCadence {
    simple_server::task_drivers::PollCadence {
        worked: Duration::from_secs(2),
        idle: Duration::from_secs(2),
        failed: Duration::from_secs(2),
    }
}
