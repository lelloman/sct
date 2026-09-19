use sct_core::{
    catalog::Catalog,
    roots::{BackendSetup, PoolSetup},
};
use sct_server::{api::App, config::Config};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        let app = App {
            writer: writer.clone(),
            registry,
            oidc,
            public_url: config.public_url.clone(),
            secure_cookies: config.public_url.starts_with("https://"),
        };
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        println!("ScT listening on {}", listener.local_addr()?);
        let lease = writer.clone();
        simple_server::axum::serve(listener,sct_server::application(app,config.web_dir)).with_graceful_shutdown(async move {
            tokio::select!{
                _=shutdown()=>{},
                _=async{loop{tokio::time::sleep(std::time::Duration::from_secs(10)).await;if lease.renew().await.is_err(){eprintln!("writer lease lost; stopping server");break;}}}=>{}
            }
        }).await?;
        writer.release().await?;
        catalog.close().await;
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
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "ScT development scaffold listening on {} (set SCT_CONFIG for foundation service)",
        listener.local_addr()?
    );
    simple_server::axum::serve(listener, sct_server::router(None, web))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
async fn local_command(
    writer: &sct_core::lease::Writer,
    args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
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
fn require_web(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if !path.join("index.html").is_file() {
        return Err("web build missing; run npm --prefix web run build or set SCT_WEB_DIR".into());
    }
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("install shutdown handler");
}
