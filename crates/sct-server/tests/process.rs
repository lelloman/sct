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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn transfer_job_survives_server_process_crash() {
    use sct_client::{Client, UploadJob, protocol::*};
    let (catalog, pool) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let owner = actor.principal_id().to_owned();
    writer.release().await.unwrap();
    let schema: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(&pool)
        .await
        .unwrap();
    let base = std::env::var("SCT_TEST_DATABASE_URL").unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{separator}options=-csearch_path%3D{schema}");
    let dir = std::env::temp_dir().join(format!(
        "sct-transfer-crash-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Crash test</title>",
    )
    .unwrap();
    let storage = dir.join("storage");
    std::fs::create_dir(&storage).unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let origin = format!("http://{address}");
    let config = dir.join("config.toml");
    std::fs::write(&config,format!("bind='{address}'\npublic_url='{origin}'\nweb_dir={:?}\ndatabase_url_env='SCT_PROCESS_DATABASE_URL'\ndevelopment=true\n[[pools]]\nid='pool'\ncapacity_bytes=1000000\noverhead_bytes=0\nstaging_bytes=100000\n[[backends]]\npool_id='pool'\n[backends.settings]\ndirectory={:?}\n[backends.inspection]\nid='local'\ndisplay_name='Local'\nadapter_kind='local'\nobserved_health='unknown'\n[backends.inspection.hints]\n",dir.to_str().unwrap(),storage.to_str().unwrap())).unwrap();
    let command = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_sct-server"));
        c.env("SCT_CONFIG", &config)
            .env("SCT_PROCESS_DATABASE_URL", &url)
            .env_remove("SCT_BIND")
            .env_remove("SCT_WEB_DIR")
            .env_remove("SCT_PUBLIC_URL");
        c
    };
    let mut process = Process(command().stdout(Stdio::null()).spawn().unwrap());
    ready(&origin, &mut process).await;
    let client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin.secret)
        .unwrap();
    let request:CreateRootRequest=serde_json::from_value(serde_json::json!({"idempotency_key":"crash-root","owner_id":owner,"display_name":"Crash","mount_path":"/crash","placement":{"kind":"single","backend_id":"local","read_policy":{"candidates":[{"backend_id":"local","connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":100000,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":200000}]})).unwrap();
    let root = client.create_root(&request).await.unwrap();
    let root = client
        .update_root(
            &root.id,
            &UpdateRootRequest {
                expected_revision: root.revision,
                display_name: root.display_name,
                state: RootState::Enabled,
                placement: root.placement,
                quota_bytes: root.quota_bytes,
            },
        )
        .await
        .unwrap();
    let parent = client
        .resolve_path(&root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let policy = client
        .effective_policy(&PolicyScope::Root {
            root_id: root.id.clone(),
        })
        .await
        .unwrap();
    let source = dir.join("source");
    std::fs::write(&source, b"survives crash").unwrap();
    let job_dir = dir.join("job");
    let job = UploadJob::prepare(
        &source,
        &job_dir,
        FileTarget::Create {
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name: "file".parse().unwrap(),
        },
        policy.revision,
        "crash-upload".parse().unwrap(),
    )
    .unwrap();
    let status = client.create_upload(&root.id, job.request()).await.unwrap();
    let upload = status.upload.id;
    for page in std::fs::read_dir(job_dir.join("pages")).unwrap() {
        client
            .upload_page(
                &root.id,
                &upload,
                std::fs::read(page.unwrap().path()).unwrap(),
            )
            .await
            .unwrap();
    }
    while client
        .admit_upload(&root.id, &upload)
        .await
        .unwrap()
        .admission
        != AdmissionState::Admitted
    {}
    client
        .upload_chunk(&root.id, &upload, b"survives crash".to_vec())
        .await
        .unwrap();
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    drop(job);
    sqlx::query("UPDATE server_lease SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let mut restarted = Process(command().stdout(Stdio::null()).spawn().unwrap());
    ready(&origin, &mut restarted).await;
    let receipt = client
        .upload(&root.id, &UploadJob::open(&job_dir).unwrap())
        .await
        .unwrap();
    let mut bytes = Vec::new();
    client
        .download_to(&root.id, &receipt.version_id, None, &mut bytes)
        .await
        .unwrap();
    assert_eq!(bytes, b"survives crash");
    drop(restarted);
    std::fs::remove_dir_all(dir).unwrap();
}
