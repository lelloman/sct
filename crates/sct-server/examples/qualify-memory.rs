//! Linux qualification: real HTTP + SDK + local adapter, bounded streaming sinks.
#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_client::{Client, UploadJob};
use sct_core::{
    backend::Registry,
    local_store::LocalAdapter,
    roots::{BackendSetup, PoolSetup},
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use serde_json::json;
use std::{collections::BTreeMap, io::Write, sync::Arc};
#[tokio::main(worker_threads = 2)]
async fn main() {
    let size: u64 = std::env::args()
        .nth(1)
        .expect("MiB argument required")
        .parse::<u64>()
        .unwrap()
        * 1024
        * 1024;
    let dir = std::env::temp_dir().join(format!("sct-memory-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).unwrap();
    let storage = dir.join("storage");
    std::fs::create_dir(&storage).unwrap();
    let source = dir.join("source");
    let mut file = std::fs::File::create(&source).unwrap();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut seed = 0x9e3779b97f4a7c15u64;
    for _ in 0..size / buffer.len() as u64 {
        for b in &mut buffer {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *b = seed as u8;
        }
        file.write_all(&buffer).unwrap();
    }
    file.sync_all().unwrap();
    drop(file);
    drop(buffer);
    let (catalog, _pool) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    let settings = json!({"directory":storage});
    let store = registry.get("local").unwrap().open(&settings).unwrap();
    writer.configure(&registry,&[PoolSetup{id:"pool".parse().unwrap(),capacity:16_000_000_000u64.try_into().unwrap(),staging:4_000_000_000u64.try_into().unwrap()}],&[BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":"local","display_name":"Local","adapter_kind":"local","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings}]).await.unwrap();
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([("local".into(), store)]),
        2,
        86400,
    )
    .unwrap();
    let w = writer.clone();
    let e = engine.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            w.renew().await.unwrap();
            e.heartbeat().await.unwrap();
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine),
    };
    let server = tokio::spawn(async move {
        simple_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::lifecycle::Shutdown::new(),
        )
        .await
        .unwrap()
    });
    let client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin.secret)
        .unwrap();
    let request:CreateRootRequest=serde_json::from_value(json!({"idempotency_key":"memory","owner_id":actor.principal_id(),"display_name":"Memory","mount_path":"/memory","placement":{"kind":"single","backend_id":"local","read_policy":{"candidates":[{"backend_id":"local","connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":5000,"throughput_window_ms":5000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":4_000_000_000u64,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":8_000_000_000u64}]})).unwrap();
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
    let started = std::time::Instant::now();
    let job = UploadJob::prepare(
        &source,
        &dir.join("job"),
        FileTarget::Create {
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name: "file".parse().unwrap(),
        },
        policy.revision,
        "memory-upload".parse().unwrap(),
    )
    .unwrap();
    let receipt = client.upload(&root.id, &job).await.unwrap();
    let count = client
        .download_to(&root.id, &receipt.version_id, None, &mut tokio::io::sink())
        .await
        .unwrap();
    assert_eq!(count, size);
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let peak: u64 = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    println!(
        "bytes={size} chunks={} height={} peak_combined_client_server_kib={peak} ceiling_kib=262144 elapsed_seconds={:.2}",
        job.manifest().chunks.get(),
        job.manifest().height,
        started.elapsed().as_secs_f64()
    );
    assert!(peak < 262144, "bounded memory ceiling exceeded");
    if size >= 1_500_000_000 {
        assert!(
            job.manifest().height > 0,
            "large plan must exercise branch pages"
        );
    }
    server.abort();
    heartbeat.abort();
    writer.release().await.unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
