#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::{
    backend::{Registry, TestAdapter},
    roots::{BackendSetup, PoolSetup},
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use serde_json::json;
use std::sync::Arc;
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn maintenance_http_queues_admin_scan_and_exposes_scoped_read_only_status() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    let backend=BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":"one","display_name":"One","adapter_kind":"test","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings:json!({"destination":"one"})};
    writer
        .configure(
            &registry,
            &[PoolSetup {
                id: "pool".parse().unwrap(),
                capacity: 1000.try_into().unwrap(),
                staging: 100.try_into().unwrap(),
            }],
            &[backend],
        )
        .await
        .unwrap();
    let root=writer.create_root(&actor,&serde_json::from_value(json!({"idempotency_key":"root","owner_id":actor.principal_id(),"display_name":"Root","mount_path":"/","placement":{"kind":"single","backend_id":"one","read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":100,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":100}]})).unwrap()).await.unwrap();
    writer
        .create_principal(&actor, &"reader".parse().unwrap())
        .await
        .unwrap();
    let reader = writer
        .issue_token(&actor, &"reader".parse().unwrap())
        .await
        .unwrap();
    writer
        .create_grant(
            &actor,
            &CreateGrantRequest {
                root_id: root.id.clone(),
                principal_id: "reader".parse().unwrap(),
                capabilities: vec![Capability::ListRead],
            },
        )
        .await
        .unwrap();
    let checkpoint_directory =
        std::env::temp_dir().join(format!("sct-http-checkpoint-{}", uuid::Uuid::new_v4()));
    let engine = TransferEngine::new(writer.clone(), Default::default(), 1, 3600)
        .unwrap()
        .with_checkpoint_directory(checkpoint_directory.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine.clone()),
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
    let http = reqwest::Client::new();
    let request = json!({"idempotency_key":"scan","root_id":root.id,"kind":"integrity_scan"});
    let url = format!("{origin}/api/v1/admin/maintenance");
    let repair_url = format!("{origin}/api/v1/admin/roots/{}/repair", root.id.as_str());
    let repair = json!({"idempotency_key":"repair","root_id":root.id,"kind":"mirror_repair"});
    assert_eq!(
        http.post(&repair_url)
            .bearer_auth(&reader.secret)
            .json(&repair)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let unsupported = http
        .post(&repair_url)
        .bearer_auth(&admin.secret)
        .json(&repair)
        .send()
        .await
        .unwrap();
    assert!(!unsupported.status().is_success());
    assert!(
        unsupported
            .text()
            .await
            .unwrap()
            .contains("unsupported_operation")
    );
    let wrong_root = json!({"idempotency_key":"repair","root_id":"wrong","kind":"mirror_repair"});
    assert_eq!(
        http.post(&repair_url)
            .bearer_auth(&admin.secret)
            .json(&wrong_root)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth(&reader.secret)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let queued = http
        .post(&url)
        .bearer_auth(&admin.secret)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(queued.status(), 200);
    let queued: MaintenanceJob = queued.json().await.unwrap();
    assert_eq!(queued.state, MaintenanceState::Queued);
    engine.tick().await.unwrap();
    let status = http
        .get(format!(
            "{origin}/api/v1/roots/{}/maintenance/{}",
            root.id.as_str(),
            queued.id.as_str()
        ))
        .bearer_auth(&reader.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    assert_eq!(
        status.json::<MaintenanceJob>().await.unwrap().state,
        MaintenanceState::Completed
    );
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/roots/wrong/maintenance/{}",
            queued.id.as_str()
        ))
        .bearer_auth(&reader.secret)
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    let findings = http
        .get(format!(
            "{origin}/api/v1/roots/{}/integrity-findings?limit=1",
            root.id.as_str()
        ))
        .bearer_auth(&reader.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(findings.status(), 200);
    assert!(
        findings
            .json::<IntegrityFindingPage>()
            .await
            .unwrap()
            .findings
            .is_empty()
    );
    assert_eq!(
        http.get(format!("{url}?limit=1"))
            .bearer_auth(&reader.secret)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let checkpoint_url = format!("{origin}/api/v1/admin/checkpoints");
    let create = json!({"idempotency_key":"checkpoint"});
    assert_eq!(
        http.post(&checkpoint_url)
            .bearer_auth(&reader.secret)
            .json(&create)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = http
        .post(&checkpoint_url)
        .bearer_auth(&admin.secret)
        .json(&create)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    let checkpoint: CheckpointInspection = response.json().await.unwrap();
    let artifact_url = format!(
        "{checkpoint_url}/{}/artifacts/catalog.jsonl",
        checkpoint.id.as_str()
    );
    assert!(
        !http
            .get(&artifact_url)
            .bearer_auth(&admin.secret)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    for _ in 0..5 {
        engine.tick().await.unwrap();
        if writer
            .checkpoint(&actor, &checkpoint.id)
            .await
            .unwrap()
            .state
            == CheckpointState::Exported
        {
            break;
        }
    }
    assert_eq!(
        http.get(&artifact_url)
            .bearer_auth(&reader.secret)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let artifact = http
        .get(&artifact_url)
        .bearer_auth(&admin.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(artifact.status(), 200);
    assert_eq!(artifact.headers()["cache-control"], "no-store");
    assert!(artifact.text().await.unwrap().contains("schema_digest"));
    let status = writer.checkpoint(&actor, &checkpoint.id).await.unwrap();
    let release_url = format!("{checkpoint_url}/{}/abandon", checkpoint.id.as_str());
    let request = json!({"expected_revision":status.revision,"acknowledge_release":false});
    assert_eq!(
        http.post(&release_url)
            .bearer_auth(&admin.secret)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let request = json!({"expected_revision":status.revision,"acknowledge_release":true});
    assert_eq!(
        http.post(&release_url)
            .bearer_auth(&admin.secret)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(
        !http
            .get(&artifact_url)
            .bearer_auth(&admin.secret)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    server.abort();
    tokio::fs::remove_dir_all(checkpoint_directory)
        .await
        .unwrap();
}
