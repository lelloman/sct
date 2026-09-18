#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn ready(origin: &str, process: &mut Process) {
    for _ in 0..100 {
        if let Ok(response) = reqwest::get(format!("{origin}/health/ready")).await
            && response.status() == 204
        {
            return;
        }
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "server exited before readiness"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("server did not become ready");
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn server_process_exclusion_shutdown_crash_and_local_recovery() {
    let (_catalog, pool) = support::database().await;
    let schema: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(&pool)
        .await
        .unwrap();
    let base = std::env::var("SCT_TEST_DATABASE_URL").unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{separator}options=-csearch_path%3D{schema}");
    let dir = std::env::temp_dir().join(format!("sct-process-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Process test</title>",
    )
    .unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let origin = format!("http://{address}");
    let config = dir.join("config.toml");
    std::fs::write(&config,format!("bind='{address}'\npublic_url='{origin}'\nweb_dir={:?}\ndatabase_url_env='SCT_PROCESS_DATABASE_URL'\ndevelopment=true\n",dir.to_str().unwrap())).unwrap();
    let command = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_sct-server"));
        c.env("SCT_CONFIG", &config)
            .env("SCT_PROCESS_DATABASE_URL", &url)
            .env_remove("SCT_BIND")
            .env_remove("SCT_WEB_DIR")
            .env_remove("SCT_PUBLIC_URL");
        c
    };
    let mut first = Process(command().stdout(Stdio::null()).spawn().unwrap());
    ready(&origin, &mut first).await;
    let second = command().output().unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("writer lease"));
    assert!(
        Command::new("kill")
            .args(["-TERM", &first.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(first.0.wait().unwrap().success());
    let mut restarted = Process(command().stdout(Stdio::null()).spawn().unwrap());
    ready(&origin, &mut restarted).await;
    restarted.0.kill().unwrap();
    restarted.0.wait().unwrap();
    assert!(!command().output().unwrap().status.success());
    sqlx::query("UPDATE server_lease SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    // Recovery runs even when the configured identity provider and secret are unavailable.
    let initial = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config,format!("{initial}\n[oidc]\nissuer='http://127.0.0.1:1'\nclient_id='unavailable'\nclient_secret_env='SCT_MISSING_SECRET'\n")).unwrap();
    let recovered = command().arg("recover-admin").output().unwrap();
    assert!(recovered.status.success());
    let credential: serde_json::Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(credential["secret"].as_str().unwrap().len(), 64);
    std::fs::write(
        &config,
        format!("{initial}\ninvalid_secret='must-not-be-echoed'\n"),
    )
    .unwrap();
    let invalid = command().output().unwrap();
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("must-not-be-echoed"));
    std::fs::remove_dir_all(dir).unwrap();
}
