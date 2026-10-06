#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
#[path = "support/tree_workflow.rs"]
mod tree_workflow;
use sct_client::{Client, protocol::*};
use sct_core::{
    backend::{Registry, TestAdapter},
    payload::{MemoryStore, ObjectStore},
    roots::{BackendSetup, PoolSetup},
    transfer_io::TransferEngine,
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn sdk_tree_and_lifecycle_workflow() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    let backend=BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":"one","display_name":"one","adapter_kind":"test","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings:json!({"destination":"one"})};
    writer
        .configure(
            &registry,
            &[PoolSetup {
                id: "pool".parse().unwrap(),
                capacity: 1000000.try_into().unwrap(),
                staging: 400000.try_into().unwrap(),
            }],
            &[backend],
        )
        .await
        .unwrap();
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        60,
    )
    .unwrap();
    let listener = sct_server::web::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine),
    };
    let server = sct_spawn(async move {
        sct_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::engine_lifecycle::Shutdown::new(),
        )
        .await
        .unwrap()
    });
    let client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin.secret.clone())
        .unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let request:CreateRootRequest=serde_json::from_value(json!({"idempotency_key":"tree","owner_id":actor.principal_id(),"display_name":"Tree","mount_path":"/tree","placement":{"kind":"single","backend_id":"one","read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":100000,"default_policy":{"mode":"strict","keep_versions":3},"allocations":[{"pool_id":"pool","retained_capacity_bytes":500000}]})).unwrap();
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
    let directory = std::env::temp_dir().join(format!("sct-sdk-tree-{}", uuid::Uuid::new_v4()));
    tree_workflow::run(&client, &root.id, &directory).await;
    std::fs::remove_dir_all(directory).unwrap();
    server.abort();
}

fn sct_spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(
        simple_server::runtime::Runtime::try_current()
            .expect("test engine")
            .scope(future),
    )
}
