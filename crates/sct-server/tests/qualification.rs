//! Destructive qualification against disposable services only. Never normal CI fixtures.
#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_client::{Client, UploadJob, protocol::*};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::{Child, Command, Stdio},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::AsyncWrite;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Fixture {
    dir: PathBuf,
    config: PathBuf,
    url: String,
    db: PgPool,
    client: Client,
    owner: PrincipalId,
    process: Option<Process>,
}
fn key() -> IdempotencyKey {
    uuid::Uuid::new_v4().to_string().parse().unwrap()
}
fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} required; use scripts/qualify"))
}
impl Fixture {
    async fn new() -> Self {
        assert_eq!(required("SCT_QUALIFICATION"), "1");
        let (catalog, db) = support::database().await;
        let w = catalog.acquire_writer().await.unwrap();
        let token = w.recover_administrator().await.unwrap();
        let owner = w
            .authenticate(&token.secret, false, None)
            .await
            .unwrap()
            .principal_id()
            .to_owned();
        w.release().await.unwrap();
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&db)
            .await
            .unwrap();
        let base = required("SCT_TEST_DATABASE_URL");
        let separator = if base.contains('?') { '&' } else { '?' };
        let url = format!("{base}{separator}options=-csearch_path%3D{schema}");
        let dir =
            PathBuf::from(required("SCT_TEST_ARTIFACTS")).join(format!("case-{}", key().as_str()));
        std::fs::create_dir_all(dir.join("local")).unwrap();
        std::fs::create_dir_all(dir.join("local2")).unwrap();
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let origin = format!("http://{address}");
        let config = dir.join("config.toml");
        let mut text = format!(
            "bind='{address}'\npublic_url='{origin}'\ndatabase_url_env='SCT_QUALIFICATION_DATABASE_URL'\ndevelopment=true\nweb_dir={:?}\ncheckpoint_dir={:?}\n[[pools]]\nid='pool'\ncapacity_bytes=32000000000\noverhead_bytes=0\nstaging_bytes=4000000000\n",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"),
            dir.join("checkpoints")
        );
        for (name, path) in [
            ("local", dir.join("local")),
            ("local2", dir.join("local2")),
            ("small", PathBuf::from(required("SCT_TEST_SMALL_DISK"))),
        ] {
            text += &format!(
                "\n[[backends]]\npool_id='pool'\n[backends.settings]\ndirectory={path:?}\n[backends.inspection]\nid='{name}'\ndisplay_name='{name}'\nadapter_kind='local'\nobserved_health='unknown'\n[backends.inspection.hints]\n"
            );
        }
        for (name, endpoint) in [
            ("s3one", required("SCT_TEST_PROXY_ENDPOINT")),
            ("s3two", required("SCT_TEST_S3_ENDPOINT")),
        ] {
            text += &format!(
                "\n[[backends]]\npool_id='pool'\n[backends.settings]\nendpoint={endpoint:?}\nbucket='sct-{name}'\nregion='us-east-1'\naccess_key_env='SCT_TEST_S3_ACCESS'\nsecret_key_env='SCT_TEST_S3_SECRET'\n[backends.inspection]\nid='{name}'\ndisplay_name='{name}'\nadapter_kind='s3'\nobserved_health='unknown'\n[backends.inspection.hints]\n"
            );
        }
        std::fs::write(&config, text).unwrap();
        let client = Client::new(origin.parse().unwrap())
            .unwrap()
            .with_token(token.secret)
            .unwrap();
        let mut f = Self {
            dir,
            config,
            url,
            db,
            client,
            owner: owner.parse().unwrap(),
            process: None,
        };
        f.start().await;
        f
    }
    async fn start(&mut self) {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("server.log"))
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_sct-server"))
            .env("SCT_CONFIG", &self.config)
            .env("SCT_QUALIFICATION_DATABASE_URL", &self.url)
            .env_remove("SCT_BIND")
            .env_remove("SCT_PUBLIC_URL")
            .env_remove("SCT_WEB_DIR")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        self.process = Some(Process(child));
        for _ in 0..300 {
            if self.client.check_readiness().await.is_ok() {
                return;
            }
            assert!(
                self.process
                    .as_mut()
                    .unwrap()
                    .0
                    .try_wait()
                    .unwrap()
                    .is_none(),
                "server died; see {:?}",
                self.dir
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("readiness timeout: {:?}", self.dir);
    }
    async fn crash(&mut self) {
        let mut child = self.process.take().unwrap();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        // Only this isolated schema; accelerate the lease wait, not work-state recovery.
        sqlx::query("UPDATE server_lease SET expires_at=clock_timestamp()-interval '1 second'")
            .execute(&self.db)
            .await
            .unwrap();
        self.start().await;
    }
    async fn root(&self, names: &[&str], keep: u32) -> RootInspection {
        let candidates: Vec<_> = names.iter().map(|name| json!({"backend_id":name,"connection_timeout_ms":500,"initial_response_timeout_ms":750,"idle_timeout_ms":750,"throughput_window_ms":1000,"minimum_bytes_per_second":100,"startup_allowance_ms":750})).collect();
        let read = json!({"candidates":candidates,"overall_deadline_ms":15000});
        let placement = if names.len() == 1 {
            json!({"kind":"single","backend_id":names[0],"read_policy":read})
        } else {
            json!({"kind":"mirror","backend_ids":names,"read_policy":read})
        };
        let id = key();
        let request = serde_json::from_value(json!({"idempotency_key":id,"owner_id":self.owner,"display_name":"Qualification","mount_path":format!("/{}",id.as_str()),"placement":placement,"quota_bytes":1000000000u64,"default_policy":{"mode":"strict","keep_versions":keep},"allocations":[{"pool_id":"pool","retained_capacity_bytes":2000000000u64}]})).unwrap();
        let r = self.client.create_root(&request).await.unwrap();
        self.client
            .update_root(
                &r.id,
                &UpdateRootRequest {
                    expected_revision: r.revision,
                    display_name: r.display_name,
                    state: RootState::Enabled,
                    placement: r.placement,
                    quota_bytes: r.quota_bytes,
                },
            )
            .await
            .unwrap()
    }
    async fn job(&self, root: &RootId, source: &Path, name: &str) -> (UploadJob, PathBuf) {
        // Acknowledgment can precede retained-generation cleanup. Respect the
        // public root ownership contract before starting a different operation.
        let mut idle = false;
        for _ in 0..300 {
            match self.client.root_operation(root).await {
                Err(sct_client::Error::Server(e)) if e.code == ErrorCode::NotFound => {
                    idle = true;
                    break;
                }
                Ok(op)
                    if matches!(
                        op.state,
                        OperationState::Completed
                            | OperationState::Aborted
                            | OperationState::Failed
                    ) =>
                {
                    idle = true;
                    break;
                }
                Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                Err(e) => panic!("operation inspection: {e:?}"),
            }
        }
        assert!(
            idle,
            "root cleanup did not release ownership within 30 seconds"
        );
        let target = match self
            .client
            .resolve_path(root, &format!("/{name}").parse().unwrap())
            .await
        {
            Ok(e) => FileTarget::Replace {
                entry_id: e.id,
                expected_revision: e.revision,
            },
            Err(sct_client::Error::Server(e)) if e.code == ErrorCode::NotFound => {
                let parent = self
                    .client
                    .resolve_path(root, &"/".parse().unwrap())
                    .await
                    .unwrap();
                FileTarget::Create {
                    parent_id: parent.id,
                    expected_parent_revision: parent.revision,
                    name: name.parse().unwrap(),
                }
            }
            Err(e) => panic!("resolve: {e:?}"),
        };
        let policy = self
            .client
            .effective_policy(&PolicyScope::Root {
                root_id: root.clone(),
            })
            .await
            .unwrap();
        let path = self.dir.join(format!("job-{}", key().as_str()));
        (
            UploadJob::prepare(source, &path, target, policy.revision, key()).unwrap(),
            path,
        )
    }
    async fn admit(&self, root: &RootId, job: &UploadJob, path: &Path) -> UploadId {
        let id = self
            .client
            .create_upload(root, job.request())
            .await
            .unwrap()
            .upload
            .id;
        for file in std::fs::read_dir(path.join("pages")).unwrap() {
            self.client
                .upload_page(root, &id, std::fs::read(file.unwrap().path()).unwrap())
                .await
                .unwrap();
        }
        for _ in 0..1000 {
            let state = self.client.admit_upload(root, &id).await.unwrap();
            assert_ne!(state.admission, AdmissionState::Rejected, "{state:?}");
            if state.admission == AdmissionState::Admitted {
                return id;
            }
        }
        panic!("admission did not converge");
    }
    async fn verify(&self, root: &RootId, receipt: &CommitReceipt, expected: &(u64, String)) {
        let mut sink = HashSink::default();
        tokio::time::timeout(
            Duration::from_secs(90),
            self.client
                .download_to(root, &receipt.version_id, None, &mut sink),
        )
        .await
        .expect("download deadline")
        .unwrap();
        assert_eq!(&sink.result(), expected, "downloaded bytes differ");
    }
}
#[derive(Default)]
struct HashSink {
    hash: Sha256,
    length: u64,
}
impl HashSink {
    fn result(self) -> (u64, String) {
        (self.length, format!("{:x}", self.hash.finalize()))
    }
}
impl AsyncWrite for HashSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.hash.update(bytes);
        self.length += bytes.len() as u64;
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
fn source(path: &Path, bytes: usize, mut seed: u64) -> (u64, String) {
    let mut file = File::create(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 65536];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(buffer.len());
        for b in &mut buffer[..n] {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *b = seed as u8;
        }
        file.write_all(&buffer[..n]).unwrap();
        hash.update(&buffer[..n]);
        left -= n;
    }
    file.sync_all().unwrap();
    (bytes as u64, format!("{:x}", hash.finalize()))
}
fn hash_file(path: &Path) -> (u64, String) {
    let mut f = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut n = 0;
    let mut b = [0; 65536];
    loop {
        let c = f.read(&mut b).unwrap();
        if c == 0 {
            break;
        }
        hash.update(&b[..c]);
        n += c as u64;
    }
    (n, format!("{:x}", hash.finalize()))
}
async fn mode(value: &str) -> u64 {
    let state: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/mode", required("SCT_TEST_PROXY_CONTROL")))
        .json(&json!({"mode":value}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["mode"], value);
    state["accepted"].as_u64().unwrap()
}

#[tokio::test]
#[ignore = "destructive qualification; scripts/qualify"]
async fn qualification_crash_matrix_preserves_acknowledged_history() {
    let mut f = Fixture::new().await;
    for placement in [&["local"][..], &["local", "s3one"][..]] {
        let root = f.root(placement, 3).await;
        let mut history: Vec<(CommitReceipt, (u64, String))> = Vec::new();
        for iteration in 0..15 {
            let phase = iteration % 5;
            let path = f.dir.join(format!("source-{iteration}"));
            let expected = source(&path, 2 * 1024 * 1024, 0x123456789 + iteration);
            let (job, job_path) = f.job(&root.id, &path, "backup.bin").await;
            let upload = f.admit(&root.id, &job, &job_path).await;
            if phase >= 1 {
                for (index, chunk) in chunker(File::open(&path).unwrap()).enumerate() {
                    f.client
                        .upload_chunk(&root.id, &upload, chunk.unwrap().data)
                        .await
                        .unwrap();
                    if phase == 1 && index == 0 {
                        break;
                    }
                }
            }
            if phase >= 2 {
                for _ in 0..1000 {
                    if f.client
                        .seal_upload(&root.id, &upload)
                        .await
                        .unwrap()
                        .upload
                        .state
                        == SessionState::Ready
                    {
                        break;
                    }
                }
                assert_eq!(
                    f.client
                        .upload_status(&root.id, &upload)
                        .await
                        .unwrap()
                        .upload
                        .state,
                    SessionState::Ready
                );
            }
            let mut acknowledged = if phase == 4 {
                Some(f.client.commit_upload(&root.id, &upload).await.unwrap())
            } else {
                None
            };
            let racing = if phase == 3 {
                let c = f.client.clone();
                let r = root.id.clone();
                let u = upload.clone();
                Some(tokio::spawn(async move { c.commit_upload(&r, &u).await }))
            } else {
                None
            };
            if phase == 3 {
                tokio::time::sleep(Duration::from_millis(iteration % 3)).await;
            }
            f.crash().await;
            if let Some(task) = racing
                && let Ok(Ok(receipt)) = task.await
            {
                acknowledged = Some(receipt);
            }
            if phase < 3 {
                let entry = f
                    .client
                    .resolve_path(&root.id, &"/backup.bin".parse().unwrap())
                    .await;
                if let Some((old, _)) = history.last() {
                    assert_eq!(
                        entry.unwrap().current_version_id,
                        Some(old.version_id.clone())
                    );
                } else {
                    assert!(entry.is_err());
                }
            }
            let receipt = f
                .client
                .upload(&root.id, &UploadJob::open(&job_path).unwrap())
                .await
                .unwrap();
            if let Some(ack) = acknowledged {
                assert_eq!(ack, receipt, "acknowledged commit changed after crash");
            }
            assert_eq!(
                f.client.commit_upload(&root.id, &upload).await.unwrap(),
                receipt
            );
            history.push((receipt, expected));
            for (receipt, expected) in history.iter().rev().take(3) {
                f.verify(&root.id, receipt, expected).await;
            }
            let entry = f
                .client
                .resolve_path(&root.id, &"/backup.bin".parse().unwrap())
                .await
                .unwrap();
            let versions = f
                .client
                .file_history(&root.id, &entry.id, None)
                .await
                .unwrap();
            assert_eq!(versions.versions.len(), history.len().min(3));
            let ids: Vec<_> = versions.versions.iter().map(|v| v.id.clone()).collect();
            let expected_ids: Vec<_> = history
                .iter()
                .rev()
                .take(3)
                .map(|(r, _)| r.version_id.clone())
                .collect();
            assert_eq!(ids, expected_ids);
            println!(
                "QUALIFY crash placement={placement:?} iteration={iteration} phase={phase} retained={} bytes_verified={}",
                ids.len(),
                history.len().min(3) * 2 * 1024 * 1024
            );
        }
    }
}

#[tokio::test]
#[ignore = "network fault qualification; scripts/qualify"]
async fn qualification_real_s3_fallback_and_failed_writes() {
    mode("normal").await;
    let f = Fixture::new().await;
    let root = f.root(&["s3one", "s3two"], 3).await;
    let path = f.dir.join("network-source");
    let expected = source(&path, 6 * 1024 * 1024, 12345);
    let (job, _) = f.job(&root.id, &path, "backup.bin").await;
    let receipt = f.client.upload(&root.id, &job).await.unwrap();
    for fault in ["offline", "stall", "cut", "latency", "normal"] {
        let before = mode(fault).await;
        let start = Instant::now();
        f.verify(&root.id, &receipt, &expected).await;
        let state: serde_json::Value =
            reqwest::get(format!("{}/state", required("SCT_TEST_PROXY_CONTROL")))
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
        assert!(
            state["accepted"].as_u64().unwrap() > before,
            "primary fault proxy was never exercised"
        );
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "fallback exceeded bound"
        );
        println!(
            "QUALIFY network fault={fault} elapsed_ms={}",
            start.elapsed().as_millis()
        );
    }
    let changed = f.dir.join("changed-source");
    let next = source(&changed, 1024 * 1024, 6789);
    let (job, job_path) = f.job(&root.id, &changed, "backup.bin").await;
    let upload = f.admit(&root.id, &job, &job_path).await;
    mode("offline").await;
    let failed = tokio::time::timeout(Duration::from_secs(30), f.client.upload(&root.id, &job))
        .await
        .expect("failed mirror must return bounded error");
    assert!(failed.is_err(), "mirror published with destination offline");
    let current = f
        .client
        .resolve_path(&root.id, &"/backup.bin".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(current.current_version_id, Some(receipt.version_id.clone()));
    f.verify(&root.id, &receipt, &expected).await;
    mode("normal").await;
    let recovered = f
        .client
        .upload(&root.id, &UploadJob::open(&job_path).unwrap())
        .await
        .unwrap();
    f.verify(&root.id, &recovered, &next).await;
    assert_eq!(
        f.client.commit_upload(&root.id, &upload).await.unwrap(),
        recovered
    );
}

#[tokio::test]
#[ignore = "large payload and SQLite qualification; scripts/qualify"]
async fn qualification_streaming_sqlite_reuse_and_disk_exhaustion() {
    mode("normal").await;
    let mut f = Fixture::new().await;
    let root = f.root(&["local", "s3one"], 3).await;
    let path = f.dir.join("large-source");
    let size = std::env::var("SCT_QUALIFY_MIB")
        .unwrap_or_else(|_| "128".into())
        .parse::<usize>()
        .unwrap()
        * 1024
        * 1024;
    assert!(size >= 32 * 1024 * 1024);
    let expected = source(&path, size, 0x987654321);
    let (job, _) = f.job(&root.id, &path, "large.bin").await;
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let sampled_peak = peak.clone();
    let pid = f.process.as_ref().unwrap().0.id();
    let sampler = tokio::spawn(async move {
        loop {
            if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status"))
                && let Some(kib) = status
                    .lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u64>().ok())
            {
                sampled_peak.fetch_max(kib, std::sync::atomic::Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let start = Instant::now();
    let first = f.client.upload(&root.id, &job).await.unwrap();
    f.verify(&root.id, &first, &expected).await;
    println!(
        "QUALIFY streaming bytes={size} ingest_and_verify_ms={} sampled_server_peak_rss_kib={}",
        start.elapsed().as_millis(),
        peak.load(std::sync::atomic::Ordering::Relaxed)
    );
    sampler.abort();
    let copies: i64 = sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE root_id=$1")
        .bind(root.id.as_str())
        .fetch_one(&f.db)
        .await
        .unwrap();
    let (repeat, _) = f.job(&root.id, &path, "large.bin").await;
    f.client.upload(&root.id, &repeat).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE root_id=$1")
        .bind(root.id.as_str())
        .fetch_one(&f.db)
        .await
        .unwrap();
    assert_eq!(copies, after, "identical backup created physical copies");
    let mut changed = OpenOptions::new().write(true).open(&path).unwrap();
    changed.seek(SeekFrom::Start((size / 2) as u64)).unwrap();
    changed.write_all(&[42; 4096]).unwrap();
    changed.sync_all().unwrap();
    drop(changed);
    let next = hash_file(&path);
    let (update, job_path) = f.job(&root.id, &path, "large.bin").await;
    let upload = f.admit(&root.id, &update, &job_path).await;
    let mut missing = 0u64;
    let mut cursor = None;
    loop {
        let page = f
            .client
            .missing_chunks(&root.id, &upload, cursor.as_ref())
            .await
            .unwrap();
        missing += page.chunks.iter().map(|c| c.length.get()).sum::<u64>();
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(
        missing > 0 && missing < (size / 2) as u64,
        "small edit failed chunk reuse: {missing}"
    );
    let receipt = f.client.upload(&root.id, &update).await.unwrap();
    f.crash().await;
    f.verify(&root.id, &receipt, &next).await;
    f.verify(&root.id, &first, &expected).await;
    println!("QUALIFY reuse total_bytes={size} changed_bytes=4096 missing_bytes={missing}");

    let database = f.dir.join("consumer.sqlite");
    let status=Command::new("sqlite3").arg(&database).arg("PRAGMA journal_mode=WAL; CREATE TABLE records(id INTEGER PRIMARY KEY, payload BLOB); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO records SELECT x,randomblob(512) FROM n;").stdout(Stdio::null()).status().unwrap();
    assert!(status.success());
    for round in 0..3 {
        let snapshot = f.dir.join(format!("snapshot-{round}.sqlite"));
        assert!(
            Command::new("sqlite3")
                .arg(&database)
                .arg(format!(".backup '{}'", snapshot.display()))
                .status()
                .unwrap()
                .success()
        );
        let (job, _) = f.job(&root.id, &snapshot, "database.sqlite").await;
        let receipt = f.client.upload(&root.id, &job).await.unwrap();
        f.verify(&root.id, &receipt, &hash_file(&snapshot)).await;
        let downloaded = f.dir.join(format!("restored-{round}.sqlite"));
        let mut output = tokio::fs::File::create(&downloaded).await.unwrap();
        f.client
            .download_to(&root.id, &receipt.version_id, None, &mut output)
            .await
            .unwrap();
        drop(output);
        let result = Command::new("sqlite3")
            .arg(&downloaded)
            .arg("PRAGMA integrity_check; SELECT count(*) FROM records;")
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(String::from_utf8(result.stdout).unwrap(), "ok\n10000\n");
        assert!(
            Command::new("sqlite3")
                .arg(&database)
                .arg("UPDATE records SET payload=randomblob(512) WHERE id % 100 = 0;")
                .status()
                .unwrap()
                .success()
        );
        println!("QUALIFY sqlite snapshot={round} integrity=ok rows=10000");
    }

    let disk = PathBuf::from(required("SCT_TEST_SMALL_DISK"));
    assert!(
        disk.starts_with("/fault-disk"),
        "only the dedicated tmpfs may be exhausted"
    );
    let r = f.root(&["small"], 3).await;
    let small = f.dir.join("small-source");
    let original = source(&small, 1024 * 1024, 2222);
    let (job, _) = f.job(&r.id, &small, "backup.bin").await;
    let committed = f.client.upload(&r.id, &job).await.unwrap();
    fn exhaust(disk: &Path) -> (PathBuf, usize) {
        let filler = disk.join("qualification-filler");
        let mut fill = File::create(&filler).unwrap();
        let block = [0; 65536];
        let mut filled = 0;
        loop {
            match fill.write_all(&block) {
                Ok(()) => {
                    filled += block.len();
                    assert!(filled <= 256 * 1024 * 1024, "tmpfs was not bounded");
                }
                Err(e) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::StorageFull);
                    break;
                }
            }
        }
        drop(fill);
        (filler, filled)
    }
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE root_id=$1")
        .bind(r.id.as_str())
        .fetch_one(&f.db)
        .await
        .unwrap();
    let (filler, filled) = exhaust(&disk);
    source(&small, 2 * 1024 * 1024, 3333);
    let (job, path) = f.job(&r.id, &small, "backup.bin").await;
    let result = tokio::time::timeout(Duration::from_secs(30), f.client.upload(&r.id, &job))
        .await
        .expect("disk-full admission deadline");
    assert!(
        matches!(result,Err(sct_client::Error::Server(ref e)) if e.code==ErrorCode::DestinationCapacity),
        "wrong disk-full failure: {result:?}"
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE root_id=$1")
        .bind(r.id.as_str())
        .fetch_one(&f.db)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "admission accepted payload copies on a full filesystem"
    );
    f.verify(&r.id, &committed, &original).await;
    std::fs::remove_file(filler).unwrap();
    // Explicit abort: failed admission need not become a successful retry.
    let upload = f
        .client
        .create_upload(&r.id, job.request())
        .await
        .unwrap()
        .upload
        .id;
    f.client.abort_upload(&r.id, &upload).await.unwrap();
    let (retry, _) = f.job(&r.id, &small, "backup.bin").await;
    let receipt = f.client.upload(&r.id, &retry).await.unwrap();
    f.verify(&r.id, &receipt, &hash_file(&small)).await;
    println!(
        "QUALIFY disk_full filled_bytes={filled} old_backup_preserved=true recovery=true job={}",
        path.display()
    );
    // Exhaustion after admission models another filesystem writer consuming the
    // previously observed free space. Admission is not a physical disk lock.
    let changed = f.dir.join("post-admission-source");
    let expected = source(&changed, 4 * 1024 * 1024, 4444);
    let (job, job_path) = f.job(&r.id, &changed, "backup.bin").await;
    f.admit(&r.id, &job, &job_path).await;
    let (filler, filled) = exhaust(&disk);
    let result = tokio::time::timeout(Duration::from_secs(30), f.client.upload(&r.id, &job))
        .await
        .expect("post-admission disk-full deadline");
    assert!(
        matches!(result,Err(sct_client::Error::Server(ref e)) if e.code==ErrorCode::DestinationCapacity),
        "wrong post-admission disk-full failure: {result:?}"
    );
    f.verify(&r.id, &receipt, &hash_file(&small)).await;
    std::fs::remove_file(filler).unwrap();
    let resumed = f
        .client
        .upload(&r.id, &UploadJob::open(&job_path).unwrap())
        .await
        .unwrap();
    f.verify(&r.id, &resumed, &expected).await;
    println!(
        "QUALIFY post_admission_disk_full filled_bytes={filled} old_backup_preserved=true same_job_resumed=true"
    );
}

#[tokio::test]
#[ignore = "concurrent retention qualification; scripts/qualify"]
async fn qualification_concurrent_roots_and_readers_remain_isolated() {
    use futures_util::{StreamExt, stream};
    mode("normal").await;
    let mut f = Fixture::new().await;
    let mut roots = Vec::new();
    for _ in 0..8 {
        roots.push(f.root(&["local", "local2"], 3).await);
    }
    let mut previous: Vec<Option<(CommitReceipt, (u64, String))>> = vec![None; 8];
    for round in 0..8 {
        let cases: Vec<_> = roots
            .iter()
            .enumerate()
            .map(|(index, root)| {
                let path = f.dir.join(format!("parallel-{round}-{index}"));
                let expected = source(&path, 1024 * 1024, 123000 + round * 8 + index as u64);
                (
                    index,
                    root.id.clone(),
                    path,
                    expected,
                    previous[index].clone(),
                )
            })
            .collect();
        let results: Vec<_> = stream::iter(cases)
            .map(|(index, root, path, expected, old)| {
                let fixture = &f;
                async move {
                    let (job, _) = fixture.job(&root, &path, "backup.bin").await;
                    let reader = async {
                        if let Some((r, bytes)) = &old {
                            for _ in 0..4 {
                                fixture.verify(&root, r, bytes).await;
                            }
                        }
                    };
                    let (receipt, ()) = tokio::join!(fixture.client.upload(&root, &job), reader);
                    let receipt = receipt.unwrap();
                    fixture.verify(&root, &receipt, &expected).await;
                    (index, receipt, expected)
                }
            })
            .buffer_unordered(8)
            .collect()
            .await;
        for (index, receipt, expected) in results {
            previous[index] = Some((receipt, expected));
        }
        f.crash().await;
        for (index, root) in roots.iter().enumerate() {
            let (receipt, expected) = previous[index].as_ref().unwrap();
            f.verify(&root.id, receipt, expected).await;
            let mut sink = HashSink::default();
            let other = &roots[(index + 1) % roots.len()].id;
            assert!(
                f.client
                    .download_to(other, &receipt.version_id, None, &mut sink)
                    .await
                    .is_err(),
                "cross-root version ID accepted"
            );
            assert_eq!(sink.length, 0, "cross-root bytes leaked");
            let current = f
                .client
                .resolve_path(&root.id, &"/backup.bin".parse().unwrap())
                .await
                .unwrap();
            assert_eq!(
                f.client
                    .file_history(&root.id, &current.id, None)
                    .await
                    .unwrap()
                    .versions
                    .len(),
                ((round + 1) as usize).min(3)
            );
        }
        let invalid:i64=sqlx::query_scalar("SELECT count(*) FROM allocations WHERE occupied<0 OR reserved<0 OR pending_cleanup<0 OR pending_cleanup>occupied OR occupied+reserved>capacity").fetch_one(&f.db).await.unwrap();
        assert_eq!(invalid, 0);
        let mismatches:i64=sqlx::query_scalar("SELECT count(*) FROM allocations a WHERE a.occupied<>(SELECT COALESCE(sum(c.bytes),0) FROM object_copies c WHERE c.allocation_id=a.id AND c.state<>'deleted')").fetch_one(&f.db).await.unwrap();
        assert_eq!(
            mismatches, 0,
            "physical copy charges do not match occupied allocations"
        );
        println!(
            "QUALIFY concurrency round={round} roots=8 mirrored_commits=8 restart_verified=true cross_root_reads_rejected=true"
        );
    }
}
