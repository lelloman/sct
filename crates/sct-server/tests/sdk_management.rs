#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_client::{CheckpointArtifact, Client, Error, protocol::*};
use sct_core::{backend::Registry, local_store::LocalAdapter, transfer_io::TransferEngine};
use serde_json::json;
use std::sync::Arc;
fn list(limit: u64) -> ListRequest {
    ListRequest {
        limit: limit.try_into().unwrap(),
        cursor: None,
    }
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn standalone_sdk_manages_accounts_capacity_grants_jobs_and_verified_checkpoint_downloads() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let token = writer.recover_administrator().await.unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    writer.configure(&registry, &[], &[]).await.unwrap();
    let directory =
        std::env::temp_dir().join(format!("sct-sdk-management-{}", uuid::Uuid::new_v4()));
    let payloads = directory.join("payloads");
    std::fs::create_dir_all(&payloads).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&payloads, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let exports = directory.join("checkpoints");
    let engine = TransferEngine::new(writer.clone(), Default::default(), 2, 3600)
        .unwrap()
        .with_checkpoint_directory(exports.clone());
    let listener = sct_server::web::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine.clone()),
    };
    let server = sct_spawn(async move {
        sct_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::engine_lifecycle::Shutdown::new(),
        )
        .await
        .unwrap();
    });
    let admin = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(token.secret)
        .unwrap();
    admin.check_readiness().await.unwrap();
    assert!(admin.account().await.unwrap().administrator);
    let principal = admin
        .create_principal(&CreatePrincipalRequest {
            id: "sdk-user".parse().unwrap(),
        })
        .await
        .unwrap();
    let first = admin.list_principals(&list(1)).await.unwrap();
    assert_eq!(first.principals.len(), 1);
    assert!(first.next_cursor.is_some());
    let next = admin
        .list_principals(&ListRequest {
            cursor: first.next_cursor,
            ..list(1)
        })
        .await
        .unwrap();
    assert_eq!(next.principals.len(), 1);
    let issued = admin.issue_credential(&principal.id).await.unwrap();
    let user = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(issued.secret.clone())
        .unwrap();
    assert_eq!(user.account().await.unwrap().principal_id, principal.id);
    assert!(
        !serde_json::to_string(
            &admin
                .list_credentials(&principal.id, &list(100))
                .await
                .unwrap()
        )
        .unwrap()
        .contains(&issued.secret)
    );
    let pool = admin
        .create_pool(&CreatePoolRequest {
            id: "sdk-pool".parse().unwrap(),
            capacity_bytes: 100000.try_into().unwrap(),
            staging_bytes: 20000.try_into().unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(admin.list_pools(&list(100)).await.unwrap().pools.len(), 1);
    assert_eq!(admin.inspect_pool(&pool.id).await.unwrap(), pool);
    let pool = admin
        .update_pool(
            &pool.id,
            &UpdatePoolRequest {
                expected_revision: pool.revision,
                capacity_bytes: 120000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    let setup:ConfigureBackendRequest = serde_json::from_value(json!({"expected_revision":null,"pool_id":pool.id,"settings":{"directory":payloads},"inspection":{"id":"sdk-local","display_name":"SDK local","adapter_kind":"local","hints":{},"observed_health":"unknown"}})).unwrap();
    let backend = admin
        .configure_backend(&setup.inspection.id, &setup)
        .await
        .unwrap();
    assert_eq!(
        admin
            .backend_configuration(&backend.inspection.id)
            .await
            .unwrap(),
        backend
    );
    assert_eq!(
        admin.inspect_backend(&backend.inspection.id).await.unwrap(),
        backend.inspection
    );
    assert_eq!(
        admin
            .list_backends(&list(100))
            .await
            .unwrap()
            .backends
            .len(),
        1
    );
    assert_eq!(
        admin.adapter_documentation("local").await.unwrap().kind,
        "local"
    );
    assert!(matches!(
        admin.adapter_documentation("../private").await,
        Err(Error::Local(_))
    ));
    let request:CreateRootRequest = serde_json::from_value(json!({"idempotency_key":"sdk-root","owner_id":principal.id,"display_name":"SDK root","mount_path":"/sdk","placement":{"kind":"single","backend_id":"sdk-local","read_policy":{"candidates":[{"backend_id":"sdk-local","connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":10000,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":pool.id,"retained_capacity_bytes":50000}]})).unwrap();
    let root = admin.create_root(&request).await.unwrap();
    let managed = admin.root_management(&root.id).await.unwrap();
    let ceiling = admin
        .update_quota_ceiling(
            &root.id,
            &UpdateQuotaCeilingRequest {
                expected_revision: managed.revision,
                quota_ceiling_bytes: 20000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    user.update_root_management(
        &root.id,
        &UpdateRootManagementRequest {
            expected_revision: ceiling.revision,
            display_name: root.display_name.clone(),
            quota_bytes: 15000.try_into().unwrap(),
            expected_upload_interval_seconds: Some(60.try_into().unwrap()),
        },
    )
    .await
    .unwrap();
    let current = admin.inspect_root(&root.id).await.unwrap();
    let constrained = admin
        .set_policy_constraints(
            &root.id,
            &PolicyConstraintsRequest {
                expected_revision: current.revision,
                min_versions: 1.try_into().unwrap(),
                max_versions: 5.try_into().unwrap(),
                allow_relaxed: false,
            },
        )
        .await
        .unwrap();
    admin
        .update_root(
            &root.id,
            &UpdateRootRequest {
                expected_revision: constrained.revision,
                display_name: constrained.display_name,
                state: RootState::Enabled,
                placement: constrained.placement,
                quota_bytes: constrained.quota_bytes,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        user.root_management(&root.id)
            .await
            .unwrap()
            .backup_freshness,
        BackupFreshness::NeverCommitted
    );
    let grant = admin
        .create_grant(&CreateGrantRequest {
            root_id: root.id.clone(),
            principal_id: admin.account().await.unwrap().principal_id,
            capabilities: vec![Capability::ListRead],
        })
        .await
        .unwrap();
    assert_eq!(admin.inspect_grant(&grant.id).await.unwrap(), grant);
    assert_eq!(
        admin
            .list_grants(&root.id, &list(100))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let updated = admin
        .update_grant(
            &grant.id,
            &UpdateGrantRequest {
                expected_revision: grant.revision.clone(),
                capabilities: vec![Capability::ListRead, Capability::ManageRoot],
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(admin.update_grant(&grant.id,&UpdateGrantRequest{expected_revision:grant.revision,capabilities:vec![Capability::ListRead]}).await,Err(Error::Server(e)) if e.code==ErrorCode::RevisionConflict)
    );
    admin
        .revoke_grant(&grant.id, &updated.revision)
        .await
        .unwrap();
    let allocations = admin.list_allocations(&list(100)).await.unwrap();
    let allocation = allocations
        .items
        .iter()
        .find(|a| matches!(&a.owner,AllocationOwner::Root{root_id} if root_id==&root.id))
        .unwrap();
    assert_eq!(
        admin.inspect_allocation(&allocation.id).await.unwrap(),
        *allocation
    );
    let management = admin.allocation_management(&allocation.id).await.unwrap();
    admin
        .resize_allocation(
            &allocation.id,
            &ResizeAllocationRequest {
                expected_revision: management.revision,
                capacity_bytes: 60000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        user.root_usage(&root.id)
            .await
            .unwrap()
            .logical_retained_bytes
            .get(),
        0
    );
    assert!(
        user.list_transfer_activity(&root.id, &list(100))
            .await
            .unwrap()
            .sessions
            .is_empty()
    );
    user.list_root_jobs(&root.id, &list(100)).await.unwrap();
    user.list_root_audit(&root.id, &list(100)).await.unwrap();
    admin.list_jobs(&list(100)).await.unwrap();
    admin.list_audit(&list(100)).await.unwrap();
    let scan = admin
        .start_integrity_scan(&StartMaintenanceRequest {
            idempotency_key: "sdk-scan".parse().unwrap(),
            kind: MaintenanceKind::IntegrityScan,
            root_id: root.id.clone(),
        })
        .await
        .unwrap();
    for _ in 0..8 {
        engine.tick().await.unwrap();
        if admin
            .maintenance_job(&root.id, &scan.id)
            .await
            .unwrap()
            .state
            == MaintenanceState::Completed
        {
            break;
        }
    }
    assert_eq!(
        admin
            .maintenance_job(&root.id, &scan.id)
            .await
            .unwrap()
            .state,
        MaintenanceState::Completed
    );
    assert_eq!(
        admin.list_maintenance(&list(100)).await.unwrap().jobs.len(),
        1
    );
    assert_eq!(
        user.list_root_maintenance(&root.id, &list(100))
            .await
            .unwrap()
            .jobs
            .len(),
        1
    );
    assert!(
        user.list_integrity_findings(&root.id, &list(100))
            .await
            .unwrap()
            .findings
            .is_empty()
    );
    let checkpoint = admin
        .create_checkpoint(&CreateCheckpointRequest {
            idempotency_key: "sdk-checkpoint".parse().unwrap(),
        })
        .await
        .unwrap();
    for _ in 0..8 {
        engine.tick().await.unwrap();
        if admin.checkpoint(&checkpoint.id).await.unwrap().state == CheckpointState::Exported {
            break;
        }
    }
    let exported = admin.checkpoint(&checkpoint.id).await.unwrap();
    assert_eq!(exported.state, CheckpointState::Exported);
    assert_eq!(
        admin
            .list_checkpoints(&list(100))
            .await
            .unwrap()
            .checkpoints
            .len(),
        1
    );
    assert!(
        admin
            .checkpoint_inventory(&checkpoint.id, &list(1))
            .await
            .unwrap()
            .objects
            .is_empty()
    );
    let descriptor = admin.checkpoint_descriptor(&checkpoint.id).await.unwrap();
    assert_eq!(descriptor.package_state, RecoveryPackageState::CatalogOnly);
    for artifact in [
        CheckpointArtifact::Catalog,
        CheckpointArtifact::Inventory,
        CheckpointArtifact::Descriptor,
    ] {
        let mut output = Vec::new();
        let count = admin
            .download_checkpoint_artifact_to(&checkpoint.id, artifact, &mut output)
            .await
            .unwrap();
        assert_eq!(count, output.len() as u64);
    }
    std::fs::write(
        exports.join(checkpoint.id.as_str()).join("catalog.jsonl"),
        b"corrupted export\n",
    )
    .unwrap();
    assert!(matches!(
        admin
            .download_checkpoint_artifact_to(
                &checkpoint.id,
                CheckpointArtifact::Catalog,
                &mut Vec::new()
            )
            .await,
        Err(Error::Integrity)
    ));
    assert!(
        matches!(admin.release_checkpoint(&checkpoint.id,&ReleaseCheckpointRequest{expected_revision:exported.revision.clone(),acknowledge_release:false}).await,Err(Error::Server(e)) if e.code==ErrorCode::InvalidRequest)
    );
    admin
        .release_checkpoint(
            &checkpoint.id,
            &ReleaseCheckpointRequest {
                expected_revision: exported.revision,
                acknowledge_release: true,
            },
        )
        .await
        .unwrap();
    user.revoke_credential(&issued.id).await.unwrap();
    assert!(
        matches!(user.account().await,Err(Error::Server(e)) if e.code==ErrorCode::Unauthorized)
    );
    let principal = admin.inspect_principal(&principal.id).await.unwrap();
    assert!(
        admin
            .update_principal(
                &principal.id,
                &UpdatePrincipalRequest {
                    expected_revision: principal.revision,
                    disabled: true,
                    administrator: false
                }
            )
            .await
            .unwrap()
            .disabled
    );
    server.abort();
    std::fs::remove_dir_all(directory).unwrap();
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
