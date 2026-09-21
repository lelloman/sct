use super::*;

fn page() -> ListRequest {
    ListRequest {
        limit: 100.try_into().unwrap(),
        cursor: None,
    }
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn storage_budgets_and_configuration_survive_bootstrap_replay() {
    let (w, a, db) = setup().await;
    let r = root(&w, &a, "/storage", 4000).await;
    let p = w.inspect_pool(&a, &"pool".parse().unwrap()).await.unwrap();
    let enlarged = w
        .update_pool(
            &a,
            &p.id,
            &UpdatePoolRequest {
                expected_revision: p.revision.clone(),
                capacity_bytes: 12000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(
        w.update_pool(
            &a,
            &p.id,
            &UpdatePoolRequest {
                expected_revision: p.revision,
                capacity_bytes: 12000.try_into().unwrap()
            }
        )
        .await
        .is_err()
    );
    let allocation = w
        .list_allocations(&a, &page())
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|x| matches!(x.owner, AllocationOwner::Root { .. }))
        .unwrap();
    let revision = w
        .allocation_management(&a, &allocation.id)
        .await
        .unwrap()
        .revision;
    let resized = w
        .resize_allocation(
            &a,
            &allocation.id,
            &ResizeAllocationRequest {
                expected_revision: revision,
                capacity_bytes: 5000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(
        w.resize_allocation(
            &a,
            &allocation.id,
            &ResizeAllocationRequest {
                expected_revision: resized.revision.clone(),
                capacity_bytes: 3999.try_into().unwrap()
            }
        )
        .await
        .is_err()
    );
    let staged = w
        .list_allocations(&a, &page())
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|x| matches!(x.owner, AllocationOwner::ServerStaging))
        .unwrap();
    sqlx::query("UPDATE allocations SET reserved=10,occupied=20,pending_cleanup=5 WHERE id=$1")
        .bind(staged.id.as_str())
        .execute(&db)
        .await
        .unwrap();
    let revision = w
        .allocation_management(&a, &staged.id)
        .await
        .unwrap()
        .revision;
    assert!(
        w.resize_allocation(
            &a,
            &staged.id,
            &ResizeAllocationRequest {
                expected_revision: revision,
                capacity_bytes: 29.try_into().unwrap()
            }
        )
        .await
        .is_err()
    );
    assert!(
        w.update_pool(
            &a,
            &p.id,
            &UpdatePoolRequest {
                expected_revision: enlarged.revision,
                capacity_bytes: 5999.try_into().unwrap()
            }
        )
        .await
        .is_err()
    );
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    let mut backend = w
        .backend_configuration(&a, &"one".parse().unwrap())
        .await
        .unwrap();
    backend.inspection.display_name = "Managed destination".into();
    let change = ConfigureBackendRequest {
        expected_revision: Some(backend.revision.clone()),
        inspection: backend.inspection.clone(),
        pool_id: backend.pool_id.clone(),
        settings: json!({"destination":"one","offline":true}),
    };
    let configured = w.configure_backend(&a, &registry, &change).await.unwrap();
    assert_eq!(
        configured.inspection.observed_health,
        ObservedHealth::Unavailable
    );
    assert!(w.configure_backend(&a, &registry, &change).await.is_err());
    let forbidden = ConfigureBackendRequest {
        expected_revision: Some(configured.revision.clone()),
        settings: json!({"destination":"different"}),
        ..change.clone()
    };
    assert_eq!(
        w.configure_backend(&a, &registry, &forbidden)
            .await
            .unwrap_err()
            .0,
        ErrorCode::UnsupportedOperation
    );
    let duplicate = ConfigureBackendRequest {
        expected_revision: None,
        inspection: BackendInspection {
            id: "alias".parse().unwrap(),
            ..backend.inspection.clone()
        },
        ..change
    };
    assert!(
        w.configure_backend(&a, &registry, &duplicate)
            .await
            .is_err()
    );
    // Even an old/missing bootstrap definition cannot overwrite managed state.
    w.configure(
        &registry,
        &[PoolSetup {
            id: p.id.clone(),
            capacity: 10.try_into().unwrap(),
            staging: 0.try_into().unwrap(),
        }],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        w.inspect_pool(&a, &p.id)
            .await
            .unwrap()
            .allocatable_bytes
            .get(),
        12000
    );
    assert_eq!(
        w.backend_configuration(&a, &"one".parse().unwrap())
            .await
            .unwrap(),
        configured
    );
    assert_eq!(
        w.inspect_allocation(&a, &allocation.id)
            .await
            .unwrap()
            .capacity_bytes
            .get(),
        5000
    );
    assert_eq!(
        w.inspect_allocation(&a, &staged.id)
            .await
            .unwrap()
            .pending_cleanup_bytes
            .get(),
        5
    );
    assert_eq!(
        w.inspect_root(&a, &r.id).await.unwrap().quota_bytes.get(),
        4000
    );
    assert_eq!(
        w.list_backends(
            &a,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: None
            }
        )
        .await
        .unwrap()
        .backends
        .len(),
        1
    );
    w.create_principal(&a, &"user".parse().unwrap())
        .await
        .unwrap();
    let token = w.issue_token(&a, &"user".parse().unwrap()).await.unwrap();
    let user = w.authenticate(&token.secret, false, None).await.unwrap();
    assert_eq!(
        w.list_pools(&user, &page()).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        w.backend_configuration(&user, &"one".parse().unwrap())
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn runtime_local_destination_accepts_bytes_and_populated_root_keeps_references() {
    use sct_core::{local_store::LocalAdapter, transfer_io::TransferEngine};
    let (w, a, db) = setup().await;
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    let dir = std::env::temp_dir().join(format!("sct-managed-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let engine = TransferEngine::new(w.clone(), Default::default(), 2, 3600).unwrap();
    let inspection:BackendInspection=serde_json::from_value(json!({"id":"local","display_name":"Local","adapter_kind":"local","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap();
    engine
        .configure_backend(
            &a,
            &registry,
            &ConfigureBackendRequest {
                expected_revision: None,
                inspection,
                pool_id: "pool".parse().unwrap(),
                settings: json!({"directory":dir}),
            },
        )
        .await
        .unwrap();
    let mut request = request(a.principal_id(), "/runtime", 4000);
    if let Placement::Single {
        backend_id,
        read_policy,
    } = &mut request.placement
    {
        *backend_id = "local".parse().unwrap();
        read_policy.candidates[0].backend_id = backend_id.clone();
    }
    let r = w.create_root(&a, &request).await.unwrap();
    let r = w
        .update_root(
            &a,
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
        .unwrap();
    let upload = ready_bytes(
        &engine,
        &w,
        &a,
        &r.id,
        "file",
        b"stored through live configuration",
    )
    .await
    .unwrap();
    engine.commit_upload(&a, &r.id, &upload).await.unwrap();
    engine.tick().await.unwrap();
    let r = w.inspect_root(&a, &r.id).await.unwrap();
    w.update_root(
        &a,
        &r.id,
        &UpdateRootRequest {
            expected_revision: r.revision,
            display_name: "Updated".parse().unwrap(),
            state: r.state,
            placement: r.placement,
            quota_bytes: r.quota_bytes,
        },
    )
    .await
    .unwrap();
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM object_copies WHERE state='available'")
            .fetch_one(&db)
            .await
            .unwrap()
            > 0
    );
    let loaded = w.configured_backends().await.unwrap();
    assert!(
        loaded
            .iter()
            .any(|b| b.inspection.id.as_str() == "local" && b.settings["directory"] == json!(dir))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn ledger_counters_do_not_invert_pool_capacity_lock_order() {
    let (_w, _a, db) = setup().await;
    let mut capacity = db.begin().await.unwrap();
    sqlx::query("SELECT id FROM pools WHERE id='pool' FOR UPDATE")
        .fetch_one(&mut *capacity)
        .await
        .unwrap();
    let mut ledger = db.begin().await.unwrap();
    // A different root's publication can hold the pool while waiting for the
    // shared staging row. Charging bytes must not acquire the pool in reverse.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query(
            "UPDATE allocations SET reserved=reserved+1 WHERE pool_id='pool' AND root_id IS NULL",
        )
        .execute(&mut *ledger),
    )
    .await
    .expect("counter update waited for unrelated pool lock")
    .unwrap();
    assert_eq!(result.rows_affected(), 1);
    ledger.rollback().await.unwrap();
    capacity.rollback().await.unwrap();
    // Budget edits must still invoke the guard; counters bypass only that guard.
    assert!(
        sqlx::query(
            "UPDATE allocations SET capacity=20000 WHERE pool_id='pool' AND root_id IS NULL"
        )
        .execute(&db)
        .await
        .is_err()
    );
}
