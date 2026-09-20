#[path = "support/deletion_tests.rs"]
mod deletion_tests;
#[path = "support/graph_work_tests.rs"]
mod graph_work_tests;
#[path = "support/inspection_tests.rs"]
mod inspection_tests;
#[path = "support/integrity_tests.rs"]
mod integrity_tests;
#[path = "support/lifecycle_tests.rs"]
mod lifecycle_tests;
#[path = "support/maintenance_tests.rs"]
mod maintenance_tests;
#[path = "support/management_tests.rs"]
mod management_tests;
#[path = "support/move_tests.rs"]
mod move_tests;
#[path = "support/policy_preview_tests.rs"]
mod policy_preview_tests;
#[path = "support/storage_management_tests.rs"]
mod storage_management_tests;
mod support;
#[path = "support/tree_build_tests.rs"]
mod tree_build_tests;
#[path = "support/tree_catalog_tests.rs"]
mod tree_catalog_tests;
#[path = "support/tree_draft_tests.rs"]
mod tree_draft_tests;
#[path = "support/tree_publication_tests.rs"]
mod tree_publication_tests;
#[path = "support/tree_read_tests.rs"]
mod tree_read_tests;
use sct_core::{
    auth::Actor,
    backend::{Registry, TestAdapter},
    lease::Writer,
    roots::{BackendSetup, PoolSetup},
};
use sct_protocol::*;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;

async fn setup() -> (Writer, Actor, PgPool) {
    let (catalog, pool) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let secret = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&secret.secret, false, None)
        .await
        .unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(TestAdapter)).unwrap();
    let backends=["one","two"].map(|b|BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":b,"display_name":b,"adapter_kind":"test","hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings:json!({"destination":b})});
    writer
        .configure(
            &registry,
            &[PoolSetup {
                id: "pool".parse().unwrap(),
                capacity: 10000.try_into().unwrap(),
                staging: 1000.try_into().unwrap(),
            }],
            &backends,
        )
        .await
        .unwrap();
    (writer, actor, pool)
}
fn request(owner: &str, mount: &str, capacity: u64) -> CreateRootRequest {
    serde_json::from_value(json!({"idempotency_key":uuid::Uuid::new_v4().simple().to_string(),"owner_id":owner,"display_name":"Storage","mount_path":mount,"placement":{"kind":"single","backend_id":"one","read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":capacity,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":capacity}]})).unwrap()
}
async fn root(writer: &Writer, actor: &Actor, mount: &str, capacity: u64) -> RootInspection {
    let root = writer
        .create_root(actor, &request(actor.principal_id(), mount, capacity))
        .await
        .unwrap();
    writer
        .update_root(
            actor,
            &root.id,
            &UpdateRootRequest {
                expected_revision: root.revision.clone(),
                display_name: root.display_name.clone(),
                state: RootState::Enabled,
                placement: root.placement.clone(),
                quota_bytes: root.quota_bytes,
            },
        )
        .await
        .unwrap()
}
async fn mkdir(
    writer: &Writer,
    actor: &Actor,
    root: &RootId,
    parent: &EntryId,
    name: &str,
    mode: VersioningMode,
) -> EntryInspection {
    let parent = writer.inspect_entry(actor, parent).await.unwrap();
    writer
        .create_directory(
            actor,
            root,
            &CreateDirectoryRequest {
                idempotency_key: uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
                parent_id: parent.id,
                expected_parent_revision: parent.revision,
                name: name.parse().unwrap(),
                versioning: mode,
            },
        )
        .await
        .unwrap()
}
async fn rename_request(
    writer: &Writer,
    actor: &Actor,
    entry: &EntryInspection,
    destination: &EntryId,
    name: &str,
) -> RenameEntryRequest {
    let entry = writer.inspect_entry(actor, &entry.id).await.unwrap();
    let source = writer
        .inspect_entry(actor, entry.parent_id.as_ref().unwrap())
        .await
        .unwrap();
    let destination = writer.inspect_entry(actor, destination).await.unwrap();
    RenameEntryRequest {
        expected_revision: entry.revision,
        expected_source_parent_revision: source.revision,
        destination_parent_id: destination.id,
        expected_destination_parent_revision: destination.revision,
        name: name.parse().unwrap(),
    }
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn credentials_identity_revocation_and_grant_boundaries() {
    let (w, admin, pool) = setup().await;
    w.create_principal(&admin, &"alice".parse().unwrap())
        .await
        .unwrap();
    w.create_principal(&admin, &"bob".parse().unwrap())
        .await
        .unwrap();
    let token = w
        .issue_token(&admin, &"alice".parse().unwrap())
        .await
        .unwrap();
    let alice = w.authenticate(&token.secret, false, None).await.unwrap();
    let own = w
        .create_root(&admin, &request("alice", "/", 4000))
        .await
        .unwrap();
    let private = root(&w, &admin, "/private", 4000).await;
    assert!(w.inspect_root(&alice, &private.id).await.is_err());
    assert!(
        w.create_root(&alice, &request("alice", "/x", 1))
            .await
            .is_err()
    );
    let g = w
        .create_grant(
            &admin,
            &CreateGrantRequest {
                root_id: private.id.clone(),
                principal_id: "alice".parse().unwrap(),
                capabilities: vec![Capability::ListRead, Capability::ManageGrants],
            },
        )
        .await
        .unwrap();
    assert!(w.inspect_root(&alice, &private.id).await.is_ok());
    assert!(
        w.create_grant(
            &alice,
            &CreateGrantRequest {
                root_id: private.id.clone(),
                principal_id: "bob".parse().unwrap(),
                capabilities: vec![Capability::Purge]
            }
        )
        .await
        .is_err()
    );
    assert!(
        w.update_grant(
            &admin,
            &g.id,
            &UpdateGrantRequest {
                expected_revision: "stale".parse().unwrap(),
                capabilities: vec![Capability::Write]
            }
        )
        .await
        .is_err()
    );
    w.revoke_grant(&admin, &g.id, &g.revision).await.unwrap();
    assert!(w.inspect_root(&alice, &private.id).await.is_err());
    let rows: Vec<String> = sqlx::query_scalar("SELECT secret_hash FROM credentials")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!rows.contains(&token.secret));
    w.revoke_credential(&admin, &token.id).await.unwrap();
    assert!(w.authenticate(&token.secret, false, None).await.is_err());
    assert!(w.inspect_root(&alice, &own.id).await.is_err());
    let session = w.oidc_session("https://issuer-a", "subject").await.unwrap();
    let same = w.oidc_session("https://issuer-a", "subject").await.unwrap();
    let other = w.oidc_session("https://issuer-b", "subject").await.unwrap();
    let a = w
        .authenticate(&session.secret, true, session.csrf.as_deref())
        .await
        .unwrap();
    let b = w.authenticate(&same.secret, true, None).await.unwrap();
    let c = w.authenticate(&other.secret, true, None).await.unwrap();
    assert_eq!(a.principal_id(), b.principal_id());
    assert_ne!(a.principal_id(), c.principal_id());
    assert!(w.authenticate(&session.secret, false, None).await.is_err());
    assert!(
        w.authenticate(&session.secret, true, Some("wrong"))
            .await
            .is_err()
    );
    w.revoke_session(&a, &session.secret).await.unwrap();
    assert!(w.authenticate(&session.secret, true, None).await.is_err());
    w.save_oidc_flow("state", "browser", "nonce", "verifier")
        .await
        .unwrap();
    assert!(w.consume_oidc_flow("state", "other-browser").await.is_err());
    assert_eq!(
        w.consume_oidc_flow("state", "browser").await.unwrap(),
        ("nonce".into(), "verifier".into())
    );
    assert!(w.consume_oidc_flow("state", "browser").await.is_err());
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn roots_allocation_idempotency_and_mirror_accounting() {
    let (w, a, pool) = setup().await;
    let req = request(a.principal_id(), "/a", 5000);
    let first = w.create_root(&a, &req).await.unwrap();
    assert_eq!(w.create_root(&a, &req).await.unwrap(), first);
    let mut changed = req.clone();
    changed.quota_bytes = 4999.try_into().unwrap();
    assert_eq!(
        w.create_root(&a, &changed).await.unwrap_err().0,
        ErrorCode::IdempotencyConflict
    );
    assert!(
        w.create_root(&a, &request(a.principal_id(), "/a/child", 10))
            .await
            .is_err()
    );
    let second = root(&w, &a, "/b", 4000).await;
    assert_eq!(
        w.inspect_pool(&a, &"pool".parse().unwrap())
            .await
            .unwrap()
            .allocated_bytes
            .get(),
        10000
    );
    assert!(
        w.create_root(&a, &request(a.principal_id(), "/c", 1))
            .await
            .is_err()
    );
    let result = w
        .list_roots(
            &a,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    let next = w
        .list_roots(
            &a,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: result.next_cursor,
            },
        )
        .await
        .unwrap();
    assert_eq!(next.items.len(), 1);
    assert_ne!(result.items[0].id, next.items[0].id);
    let update = UpdateRootRequest {
        expected_revision: first.revision.clone(),
        display_name: first.display_name.clone(),
        state: RootState::Enabled,
        placement: first.placement.clone(),
        quota_bytes: first.quota_bytes,
    };
    assert_eq!(
        w.update_root(&a, &first.id, &update).await.unwrap().state,
        RootState::Enabled
    );
    assert!(w.update_root(&a, &first.id, &update).await.is_err());
    sqlx::query("UPDATE roots SET populated=true WHERE id=$1")
        .bind(second.id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    let mut other = second.placement.clone();
    if let Placement::Single {
        backend_id,
        read_policy,
    } = &mut other
    {
        *backend_id = "two".parse().unwrap();
        read_policy.candidates[0].backend_id = backend_id.clone();
    }
    assert!(
        w.update_root(
            &a,
            &second.id,
            &UpdateRootRequest {
                expected_revision: second.revision,
                display_name: second.display_name,
                state: RootState::Enabled,
                placement: other,
                quota_bytes: second.quota_bytes
            }
        )
        .await
        .is_err()
    );
    let (w, a, _) = setup().await;
    let mut req = request(a.principal_id(), "/mirror", 4000);
    let Placement::Single { read_policy, .. } = req.placement else {
        unreachable!()
    };
    let mut reads = read_policy.clone();
    let mut second = reads.candidates[0].clone();
    second.backend_id = "two".parse().unwrap();
    reads.candidates.push(second);
    req.placement = Placement::Mirror {
        backend_ids: ["one".parse().unwrap(), "two".parse().unwrap()],
        read_policy: reads,
    };
    assert_eq!(
        w.create_root(&a, &req).await.unwrap_err().0,
        ErrorCode::QuotaExhausted
    );
    req.allocations[0].retained_capacity_bytes = 8000.try_into().unwrap();
    assert!(w.create_root(&a, &req).await.is_ok());
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn durable_ownership_recovery_and_shared_staging() {
    let (w, a, pool) = setup().await;
    let one = root(&w, &a, "/one", 4000).await;
    let two = root(&w, &a, "/two", 4000).await;
    let op = w
        .begin_operation(&a, &one.id, OperationKind::Upload)
        .await
        .unwrap();
    assert!(
        w.begin_operation(&a, &one.id, OperationKind::Namespace)
            .await
            .is_err()
    );
    let other = w
        .begin_operation(&a, &two.id, OperationKind::Upload)
        .await
        .unwrap();
    w.reserve_staging(&op, &"pool".parse().unwrap(), 600.try_into().unwrap())
        .await
        .unwrap();
    assert_eq!(
        w.reserve_staging(&other, &"pool".parse().unwrap(), 500.try_into().unwrap())
            .await
            .unwrap_err()
            .0,
        ErrorCode::StagingExhausted
    );
    assert!(w.finish_operation(&op, false).await.is_err());
    assert!(
        w.resolve_path(&a, &one.id, &"/".parse().unwrap())
            .await
            .is_ok()
    );
    sqlx::query(
        "UPDATE operations SET worker_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(op.id().as_str())
    .execute(&pool)
    .await
    .unwrap();
    let recovered = w.recover_operation(&op.id()).await.unwrap();
    assert!(w.finish_operation(&op, true).await.is_err());
    w.release_unused_reservations(&recovered).await.unwrap();
    w.finish_operation(&recovered, true).await.unwrap();
    w.reserve_staging(&other, &"pool".parse().unwrap(), 1000.try_into().unwrap())
        .await
        .unwrap();
    w.release().await.unwrap();
    let restarted = w.catalog().acquire_writer().await.unwrap();
    assert_eq!(
        restarted
            .inspect_operation(&a, &two.id)
            .await
            .unwrap()
            .state,
        OperationState::Recovering
    );
    assert!(
        restarted
            .begin_operation(&a, &two.id, OperationKind::Namespace)
            .await
            .is_err()
    );
    let reclaimed = restarted.recover_operation(&other.id()).await.unwrap();
    assert!(w.finish_operation(&other, false).await.is_err());
    restarted
        .release_unused_reservations(&reclaimed)
        .await
        .unwrap();
    restarted.finish_operation(&reclaimed, true).await.unwrap();
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn namespace_pagination_identity_and_conflicts() {
    let (w, a, _) = setup().await;
    let r = root(&w, &a, "/", 5000).await;
    let top = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let upper = mkdir(
        &w,
        &a,
        &r.id,
        &top.id,
        "Name",
        VersioningMode::IndependentFiles,
    )
    .await;
    let lower = mkdir(
        &w,
        &a,
        &r.id,
        &top.id,
        "name",
        VersioningMode::IndependentFiles,
    )
    .await;
    let unicode = mkdir(
        &w,
        &a,
        &r.id,
        &top.id,
        "caffè",
        VersioningMode::IndependentFiles,
    )
    .await;
    assert_eq!(
        w.resolve_path(&a, &r.id, &"/caffè".parse().unwrap())
            .await
            .unwrap()
            .id,
        unicode.id
    );
    let first = w
        .list_entries(
            &a,
            &r.id,
            &top.id,
            &ListEntriesRequest {
                limit: 1.try_into().unwrap(),
                cursor: None,
                include_hidden: false,
            },
        )
        .await
        .unwrap();
    let next = w
        .list_entries(
            &a,
            &r.id,
            &top.id,
            &ListEntriesRequest {
                limit: 1.try_into().unwrap(),
                cursor: first.next_cursor.clone(),
                include_hidden: false,
            },
        )
        .await
        .unwrap();
    assert_ne!(first.items[0].id, next.items[0].id);
    assert!(
        w.list_entries(
            &a,
            &r.id,
            &top.id,
            &ListEntriesRequest {
                limit: 2.try_into().unwrap(),
                cursor: first.next_cursor.clone(),
                include_hidden: false
            }
        )
        .await
        .is_err()
    );
    let req = rename_request(&w, &a, &upper, &top.id, "renamed").await;
    let (one, two) = tokio::join!(
        w.rename_entry(&a, &upper.id, &req),
        w.rename_entry(&a, &upper.id, &req)
    );
    assert_ne!(one.is_ok(), two.is_ok());
    assert_eq!(
        w.resolve_path(&a, &r.id, &"/renamed".parse().unwrap())
            .await
            .unwrap()
            .id,
        upper.id
    );
    assert_eq!(
        w.list_entries(
            &a,
            &r.id,
            &top.id,
            &ListEntriesRequest {
                limit: 1.try_into().unwrap(),
                cursor: first.next_cursor,
                include_hidden: false
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::CursorRefreshRequired
    );
    let cycle = rename_request(&w, &a, &lower, &lower.id, "cycle").await;
    assert!(w.rename_entry(&a, &lower.id, &cycle).await.is_err());
    let collision = rename_request(&w, &a, &lower, &top.id, "caffè").await;
    assert!(w.rename_entry(&a, &lower.id, &collision).await.is_err());
    let current = w.inspect_entry(&a, &top.id).await.unwrap();
    let req = CreateDirectoryRequest {
        idempotency_key: "retry".parse().unwrap(),
        parent_id: top.id,
        expected_parent_revision: current.revision,
        name: "retry".parse().unwrap(),
        versioning: VersioningMode::IndependentFiles,
    };
    let once = w.create_directory(&a, &r.id, &req).await.unwrap();
    assert_eq!(w.create_directory(&a, &r.id, &req).await.unwrap(), once);
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn effective_policies_atomic_scopes_and_transition_exclusion() {
    let (w, a, _) = setup().await;
    let r = root(&w, &a, "/", 5000).await;
    let top = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let dir = mkdir(
        &w,
        &a,
        &r.id,
        &top.id,
        "scoped",
        VersioningMode::IndependentFiles,
    )
    .await;
    let scope = PolicyScope::Entry {
        root_id: r.id.clone(),
        entry_id: dir.id.clone(),
    };
    let old = w.effective_policy(&a, &scope).await.unwrap();
    assert_eq!(old.policy, RetentionPolicy::default());
    assert!(old.inherited);
    let req = SetPolicyRequest {
        scope: scope.clone(),
        expected_revision: old.revision,
        expected_scope_revision: dir.revision.clone(),
        policy: RetentionPolicy {
            mode: RetentionMode::Strict,
            keep_versions: 3.try_into().unwrap(),
        },
        versioning: VersioningMode::IndependentFiles,
    };
    let changed = w.set_policy(&a, &req).await.unwrap();
    assert_eq!(changed.policy.keep_versions.get(), 3);
    assert!(!changed.inherited);
    assert!(w.set_policy(&a, &req).await.is_err());
    let child = mkdir(
        &w,
        &a,
        &r.id,
        &dir.id,
        "child",
        VersioningMode::IndependentFiles,
    )
    .await;
    assert_eq!(
        w.effective_policy(
            &a,
            &PolicyScope::Entry {
                root_id: r.id.clone(),
                entry_id: child.id.clone()
            }
        )
        .await
        .unwrap()
        .policy
        .keep_versions
        .get(),
        3
    );
    let moving = rename_request(&w, &a, &child, &top.id, "moved").await;
    assert!(w.rename_entry(&a, &child.id, &moving).await.is_err());
    let dir_now = w.inspect_entry(&a, &dir.id).await.unwrap();
    assert!(
        w.set_policy(
            &a,
            &SetPolicyRequest {
                scope,
                expected_revision: changed.revision,
                expected_scope_revision: dir_now.revision,
                policy: changed.policy,
                versioning: VersioningMode::AtomicDirectory
            }
        )
        .await
        .is_err()
    );
    let atomic = mkdir(
        &w,
        &a,
        &r.id,
        &top.id,
        "atomic",
        VersioningMode::AtomicDirectory,
    )
    .await;
    let bad = CreateDirectoryRequest {
        idempotency_key: "bad".parse().unwrap(),
        parent_id: atomic.id.clone(),
        expected_parent_revision: atomic.revision,
        name: "nested".parse().unwrap(),
        versioning: VersioningMode::AtomicDirectory,
    };
    assert!(w.create_directory(&a, &r.id, &bad).await.is_err());
    let op = w
        .begin_operation(&a, &r.id, OperationKind::PolicyTransition)
        .await
        .unwrap();
    assert!(
        w.effective_policy(
            &a,
            &PolicyScope::Root {
                root_id: r.id.clone()
            }
        )
        .await
        .unwrap()
        .transitioning
    );
    assert!(w.rename_entry(&a, &child.id, &moving).await.is_err());
    w.finish_operation(&op, false).await.unwrap();
    let root_now = w.inspect_root(&a, &r.id).await.unwrap();
    w.set_policy_constraints(
        &a,
        &r.id,
        &root_now.revision,
        1.try_into().unwrap(),
        3.try_into().unwrap(),
        false,
    )
    .await
    .unwrap();
    let policy = w
        .effective_policy(
            &a,
            &PolicyScope::Root {
                root_id: r.id.clone(),
            },
        )
        .await
        .unwrap();
    let root_now = w.inspect_root(&a, &r.id).await.unwrap();
    assert!(
        w.set_policy(
            &a,
            &SetPolicyRequest {
                scope: PolicyScope::Root { root_id: r.id },
                expected_revision: policy.revision,
                expected_scope_revision: root_now.revision,
                policy: RetentionPolicy {
                    mode: RetentionMode::Relaxed,
                    keep_versions: 3.try_into().unwrap()
                },
                versioning: VersioningMode::IndependentFiles
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn concurrent_pool_admission_cross_root_moves_and_cursor_isolation() {
    let (w, a, _) = setup().await;
    w.create_principal(&a, &"alice".parse().unwrap())
        .await
        .unwrap();
    w.create_principal(&a, &"bob".parse().unwrap())
        .await
        .unwrap();
    let req_a = request("alice", "/", 6000);
    let req_b = request("bob", "/", 6000);
    let (left, right) = tokio::join!(w.create_root(&a, &req_a), w.create_root(&a, &req_b));
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(
        w.inspect_pool(&a, &"pool".parse().unwrap())
            .await
            .unwrap()
            .allocated_bytes
            .get(),
        7000
    );
    let (w, a, _) = setup().await;
    let one = root(&w, &a, "/one", 4000).await;
    let two = root(&w, &a, "/two", 4000).await;
    let top = w
        .resolve_path(&a, &one.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let other = w
        .resolve_path(&a, &two.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let child = mkdir(
        &w,
        &a,
        &one.id,
        &top.id,
        "child",
        VersioningMode::IndependentFiles,
    )
    .await;
    mkdir(
        &w,
        &a,
        &one.id,
        &top.id,
        "other",
        VersioningMode::IndependentFiles,
    )
    .await;
    let cross = rename_request(&w, &a, &child, &other.id, "cross").await;
    assert!(w.rename_entry(&a, &child.id, &cross).await.is_err());
    let wrong_parent = CreateDirectoryRequest {
        idempotency_key: "cross".parse().unwrap(),
        parent_id: other.id,
        expected_parent_revision: other.revision,
        name: "cross".parse().unwrap(),
        versioning: VersioningMode::IndependentFiles,
    };
    assert!(
        w.create_directory(&a, &one.id, &wrong_parent)
            .await
            .is_err()
    );
    w.create_principal(&a, &"reader".parse().unwrap())
        .await
        .unwrap();
    let token = w.issue_token(&a, &"reader".parse().unwrap()).await.unwrap();
    let reader = w.authenticate(&token.secret, false, None).await.unwrap();
    w.create_grant(
        &a,
        &CreateGrantRequest {
            root_id: one.id.clone(),
            principal_id: "reader".parse().unwrap(),
            capabilities: vec![Capability::ListRead],
        },
    )
    .await
    .unwrap();
    let page = w
        .list_entries(
            &a,
            &one.id,
            &top.id,
            &ListEntriesRequest {
                limit: 1.try_into().unwrap(),
                cursor: None,
                include_hidden: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        w.list_entries(
            &reader,
            &one.id,
            &top.id,
            &ListEntriesRequest {
                limit: 1.try_into().unwrap(),
                cursor: page.next_cursor,
                include_hidden: false
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::CursorRefreshRequired
    );
    let disabled = w
        .create_root(&a, &request(a.principal_id(), "/disabled", 1))
        .await
        .unwrap();
    let parent = w
        .resolve_path(&a, &disabled.id, &"/".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        w.create_directory(
            &a,
            &disabled.id,
            &CreateDirectoryRequest {
                idempotency_key: "disabled".parse().unwrap(),
                parent_id: parent.id,
                expected_parent_revision: parent.revision,
                name: "blocked".parse().unwrap(),
                versioning: VersioningMode::IndependentFiles
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn transfer_graph_reservations_and_pin_retirement_orders() {
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    writer.catalog().migrate().await.unwrap();
    let root = root(&writer, &actor, "/transfer", 4000).await;
    let fence = writer
        .begin_operation(&actor, &root.id, OperationKind::Upload)
        .await
        .unwrap();
    let demand = BTreeMap::from([("pool".parse().unwrap(), 800.try_into().unwrap())]);
    writer.reserve_transfer(&fence, &demand).await.unwrap();
    writer.reserve_transfer(&fence, &demand).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT reserved FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        800
    );
    let bad = BTreeMap::from([("pool".parse().unwrap(), 900.try_into().unwrap())]);
    assert!(writer.reserve_transfer(&fence, &bad).await.is_err());
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![ChunkReference {
            digest: digest(b"abc").parse().unwrap(),
            length: 3.try_into().unwrap(),
        }],
    };
    let bytes = canonical(&page).unwrap();
    let hash = digest(&bytes);
    writer
        .store_manifest_page(&fence, &hash.parse().unwrap(), &bytes)
        .await
        .unwrap();
    writer
        .store_manifest_page(&fence, &hash.parse().unwrap(), &bytes)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM content_edges")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // Catalog constraints disallow cross-root edges and mutation of stored bytes.
    assert!(
        sqlx::query("UPDATE content_objects SET body='x'::bytea WHERE digest=$1")
            .bind(&hash)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) VALUES('other',$1,0,$1,3,1)").bind(&hash).execute(&pool).await.is_err());
    let entry = writer
        .resolve_path(&actor, &root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    // A version must point to a file manifest, not directly to a chunk page.
    let manifest = FileManifest {
        format: 1,
        profile: CHUNK_PROFILE.into(),
        length: 3.try_into().unwrap(),
        chunks: 1.try_into().unwrap(),
        height: 0,
        tree: Some(hash.parse().unwrap()),
    };
    let body = canonical(&manifest).unwrap();
    let manifest_id = digest(&body);
    sqlx::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,'file',3,0,$3)")
        .bind(root.id.as_str()).bind(&manifest_id).bind(body).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) VALUES('pin-file',$1,$2,'pin-file','file','r1')")
        .bind(root.id.as_str()).bind(entry.id.as_str()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length) VALUES('version',$1,'pin-file',$2,3)").bind(root.id.as_str()).bind(&manifest_id).execute(&pool).await.unwrap();
    let version = "version".parse().unwrap();
    let pin = writer
        .pin_version(&actor, &root.id, &version)
        .await
        .unwrap();
    let n=sqlx::query("UPDATE file_versions SET retired=true WHERE id='version' AND NOT EXISTS(SELECT 1 FROM read_pins WHERE version_id='version' AND expires_at>clock_timestamp())").execute(&pool).await.unwrap().rows_affected();
    assert_eq!(n, 0);
    writer.release_read_pin(&actor, &pin).await.unwrap();
    sqlx::query("UPDATE file_versions SET retired=true WHERE id='version'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        writer
            .pin_version(&actor, &root.id, &version)
            .await
            .is_err()
    );
    writer.release_unused_reservations(&fence).await.unwrap();
    writer.finish_operation(&fence, true).await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn physical_copy_lost_response_and_recovery_preserve_charges() {
    use sct_core::{
        payload::{Fault, MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/copies", 4000).await;
    let fence = writer
        .begin_operation(&actor, &root.id, OperationKind::Upload)
        .await
        .unwrap();
    let chunk: ChunkDigest = digest(b"abc").parse().unwrap();
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![ChunkReference {
            digest: chunk.clone(),
            length: 3.try_into().unwrap(),
        }],
    };
    let bytes = canonical(&page).unwrap();
    writer
        .store_manifest_page(&fence, &digest(&bytes).parse().unwrap(), &bytes)
        .await
        .unwrap();
    writer
        .reserve_transfer(
            &fence,
            &BTreeMap::from([("pool".parse().unwrap(), 3.try_into().unwrap())]),
        )
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]),
        2,
        86400,
    )
    .unwrap();
    memory.fault(Fault::LostWriteResponse).await;
    assert!(
        engine
            .write_copy(&fence, &chunk, &"one".parse().unwrap(), b"abc".to_vec())
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );
    assert!(writer.finish_operation(&fence, false).await.is_err());
    // A new worker uses the existing operation, not a competing root owner.
    sqlx::query("UPDATE operations SET worker_expires_at=clock_timestamp()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let recovered = writer.recover_operation(&fence.id()).await.unwrap();
    assert!(engine.reconcile_writes(&fence).await.is_err());
    memory.fault(Fault::CorruptRead).await;
    engine.reconcile_writes(&recovered).await.unwrap();
    memory.fault(Fault::UnknownDelete).await;
    assert_eq!(engine.cleanup_copies(&recovered).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );
    memory.fault(Fault::None).await;
    assert_eq!(engine.cleanup_copies(&recovered).await.unwrap(), 1);
    assert_eq!(engine.cleanup_copies(&recovered).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    writer
        .release_unused_reservations(&recovered)
        .await
        .unwrap();
    writer.finish_operation(&recovered, true).await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn metadata_first_admission_and_abort_release_only_unused_space() {
    use sct_core::{
        payload::{MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/sessions", 4000).await;
    let entry = writer
        .resolve_path(&actor, &root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let policy = writer
        .effective_policy(
            &actor,
            &PolicyScope::Root {
                root_id: root.id.clone(),
            },
        )
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([("one".into(), memory as Arc<dyn ObjectStore>)]),
        2,
        86400,
    )
    .unwrap();
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![ChunkReference {
            digest: digest(b"abc").parse().unwrap(),
            length: 3.try_into().unwrap(),
        }],
    };
    let page_bytes = canonical(&page).unwrap();
    let page_hash = digest(&page_bytes);
    let request = CreateUploadRequest {
        idempotency_key: "upload-one".parse().unwrap(),
        target: FileTarget::Create {
            parent_id: entry.id,
            expected_parent_revision: entry.revision,
            name: "file".parse().unwrap(),
        },
        expected_policy_revision: policy.revision,
        manifest: FileManifest {
            format: 1,
            profile: CHUNK_PROFILE.into(),
            length: 3.try_into().unwrap(),
            chunks: 1.try_into().unwrap(),
            height: 0,
            tree: Some(page_hash.parse().unwrap()),
        },
    };
    let first = engine
        .create_upload(&actor, &root.id, &request)
        .await
        .unwrap();
    assert_eq!(first.admission, AdmissionState::Planning);
    assert_eq!(
        engine
            .create_upload(&actor, &root.id, &request)
            .await
            .unwrap()
            .upload
            .id,
        first.upload.id
    );
    let mut changed = request.clone();
    changed.manifest.length = 4.try_into().unwrap();
    assert!(matches!(
        engine
            .create_upload(&actor, &root.id, &changed)
            .await
            .unwrap_err()
            .0,
        ErrorCode::IdempotencyConflict
    ));
    engine
        .upload_page(
            &actor,
            &root.id,
            &first.upload.id,
            &page_hash.parse().unwrap(),
            &page_bytes,
        )
        .await
        .unwrap();
    let mut admitted = false;
    for _ in 0..10 {
        if engine
            .admit_upload(&actor, &root.id, &first.upload.id)
            .await
            .unwrap()
            .admission
            == AdmissionState::Admitted
        {
            admitted = true;
            break;
        }
    }
    assert!(admitted);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM object_copies")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT reserved FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap()
            > 3
    );
    assert!(
        engine
            .upload_page(
                &actor,
                &root.id,
                &first.upload.id,
                &page_hash.parse().unwrap(),
                &page_bytes
            )
            .await
            .is_err()
    );
    assert_eq!(
        engine
            .abort_upload(&actor, &root.id, &first.upload.id)
            .await
            .unwrap()
            .upload
            .state,
        SessionState::Aborted
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT reserved FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let mut request = request;
    request.idempotency_key = "upload-no-space".parse().unwrap();
    sqlx::query("UPDATE allocations SET capacity=100 WHERE root_id IS NULL")
        .execute(&pool)
        .await
        .unwrap();
    let second = engine
        .create_upload(&actor, &root.id, &request)
        .await
        .unwrap();
    engine
        .upload_page(
            &actor,
            &root.id,
            &second.upload.id,
            &page_hash.parse().unwrap(),
            &page_bytes,
        )
        .await
        .unwrap();
    let mut failed = false;
    for _ in 0..10 {
        if let Err(error) = engine
            .admit_upload(&actor, &root.id, &second.upload.id)
            .await
        {
            assert_eq!(error.0, ErrorCode::StagingExhausted);
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert_eq!(
        engine
            .upload_status(&actor, &root.id, &second.upload.id)
            .await
            .unwrap()
            .admission,
        AdmissionState::Rejected
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM transfer_audit")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn mirrored_ingestion_retries_completed_copies_and_seals_only_verified_graph() {
    use sct_core::{
        payload::{Fault, MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/ingest", 4000).await;
    let placement:Placement=serde_json::from_value(json!({"kind":"mirror","backend_ids":["one","two"],"read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000},{"backend_id":"two","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}})).unwrap();
    let root = writer
        .update_root(
            &actor,
            &root.id,
            &UpdateRootRequest {
                expected_revision: root.revision,
                display_name: root.display_name,
                state: RootState::Enabled,
                placement,
                quota_bytes: 2000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    let entry = writer
        .resolve_path(&actor, &root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let policy = writer
        .effective_policy(
            &actor,
            &PolicyScope::Root {
                root_id: root.id.clone(),
            },
        )
        .await
        .unwrap();
    let one = Arc::new(MemoryStore::default());
    let two = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([
            ("one".into(), one as Arc<dyn ObjectStore>),
            ("two".into(), two.clone() as Arc<dyn ObjectStore>),
        ]),
        2,
        86400,
    )
    .unwrap();
    let chunk: ChunkDigest = digest(b"abc").parse().unwrap();
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![ChunkReference {
            digest: chunk.clone(),
            length: 3.try_into().unwrap(),
        }],
    };
    let page_bytes = canonical(&page).unwrap();
    let page_hash: ManifestId = digest(&page_bytes).parse().unwrap();
    let request = CreateUploadRequest {
        idempotency_key: "mirror".parse().unwrap(),
        target: FileTarget::Create {
            parent_id: entry.id,
            expected_parent_revision: entry.revision,
            name: "file".parse().unwrap(),
        },
        expected_policy_revision: policy.revision,
        manifest: FileManifest {
            format: 1,
            profile: CHUNK_PROFILE.into(),
            length: 3.try_into().unwrap(),
            chunks: 1.try_into().unwrap(),
            height: 0,
            tree: Some(page_hash.clone()),
        },
    };
    let s = engine
        .create_upload(&actor, &root.id, &request)
        .await
        .unwrap();
    let upload = s.upload.id;
    assert!(
        engine
            .chunk_permission(&actor, &root.id, &upload, &chunk)
            .await
            .is_err()
    );
    engine
        .upload_page(&actor, &root.id, &upload, &page_hash, &page_bytes)
        .await
        .unwrap();
    for _ in 0..10 {
        if engine
            .admit_upload(&actor, &root.id, &upload)
            .await
            .unwrap()
            .admission
            == AdmissionState::Admitted
        {
            break;
        }
    }
    assert!(
        engine
            .upload_chunk(&actor, &root.id, &upload, &chunk, b"bad".to_vec())
            .await
            .is_err()
    );
    two.fault(Fault::LostWriteResponse).await;
    assert!(
        engine
            .upload_chunk(&actor, &root.id, &upload, &chunk, b"abc".to_vec())
            .await
            .is_err()
    );
    let before: i64 = sqlx::query_scalar("SELECT occupied FROM allocations WHERE root_id IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, 6);
    two.fault(Fault::None).await;
    engine
        .upload_chunk(&actor, &root.id, &upload, &chunk, b"abc".to_vec())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before
    );
    assert!(
        engine
            .missing_chunks(&actor, &root.id, &upload, None)
            .await
            .unwrap()
            .chunks
            .is_empty()
    );
    let sealed = engine.seal_upload(&actor, &root.id, &upload).await.unwrap();
    assert_eq!(sealed.upload.state, SessionState::Ready);
    assert!(
        sealed
            .upload
            .copies
            .iter()
            .all(|c| c.state == CopyAcknowledgement::Acknowledged)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM file_versions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    engine
        .abort_upload(&actor, &root.id, &upload)
        .await
        .unwrap();
    engine.tick().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

async fn plan_bytes(
    engine: &sct_core::transfer_io::TransferEngine,
    writer: &Writer,
    actor: &Actor,
    root: &RootId,
    name: &str,
    bytes: &[u8],
) -> sct_core::error::Result<UploadId> {
    let parent = writer
        .resolve_path(actor, root, &"/".parse().unwrap())
        .await?;
    let existing = writer
        .resolve_path(actor, root, &format!("/{name}").parse().unwrap())
        .await;
    let (target, policy_entry) = match existing {
        Ok(e) => (
            FileTarget::Replace {
                entry_id: e.id.clone(),
                expected_revision: e.revision,
            },
            e.id,
        ),
        Err(_) => (
            FileTarget::Create {
                parent_id: parent.id.clone(),
                expected_parent_revision: parent.revision,
                name: name.parse().unwrap(),
            },
            parent.id,
        ),
    };
    let policy = writer
        .effective_policy(
            actor,
            &PolicyScope::Entry {
                root_id: root.clone(),
                entry_id: policy_entry,
            },
        )
        .await?;
    let page = ManifestPage::Chunks {
        format: 1,
        chunks: vec![ChunkReference {
            digest: digest(bytes).parse().unwrap(),
            length: (bytes.len() as u64).try_into().unwrap(),
        }],
    };
    let page_bytes = canonical(&page)?;
    let hash: ManifestId = digest(&page_bytes).parse()?;
    let request = CreateUploadRequest {
        idempotency_key: uuid::Uuid::new_v4().simple().to_string().parse()?,
        target,
        expected_policy_revision: policy.revision,
        manifest: FileManifest {
            format: 1,
            profile: CHUNK_PROFILE.into(),
            length: (bytes.len() as u64).try_into()?,
            chunks: if bytes.is_empty() { 0 } else { 1 }.try_into()?,
            height: 0,
            tree: (!bytes.is_empty()).then_some(hash.clone()),
        },
    };
    let upload = engine.create_upload(actor, root, &request).await?.upload.id;
    if !bytes.is_empty() {
        engine
            .upload_page(actor, root, &upload, &hash, &page_bytes)
            .await?;
    }
    Ok(upload)
}
async fn ready_bytes(
    engine: &sct_core::transfer_io::TransferEngine,
    writer: &Writer,
    actor: &Actor,
    root: &RootId,
    name: &str,
    bytes: &[u8],
) -> sct_core::error::Result<UploadId> {
    let upload = plan_bytes(engine, writer, actor, root, name, bytes).await?;
    let mut admitted = false;
    for _ in 0..20 {
        if engine.admit_upload(actor, root, &upload).await?.admission == AdmissionState::Admitted {
            admitted = true;
            break;
        }
    }
    assert!(admitted);
    if !bytes.is_empty() {
        engine
            .upload_chunk(
                actor,
                root,
                &upload,
                &digest(bytes).parse()?,
                bytes.to_vec(),
            )
            .await?;
    }
    let mut sealed = false;
    for _ in 0..20 {
        if engine.seal_upload(actor, root, &upload).await?.upload.state == SessionState::Ready {
            sealed = true;
            break;
        }
    }
    assert!(sealed);
    Ok(upload)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn publication_keep_one_receipts_and_pinned_outgoing_accounting() {
    use sct_core::{
        payload::{MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/publish", 4000).await;
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        86400,
    )
    .unwrap();
    let first = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"first")
        .await
        .unwrap();
    let receipt = engine
        .commit_upload(&actor, &root.id, &first)
        .await
        .unwrap();
    let pin = writer
        .pin_version(&actor, &root.id, &receipt.version_id)
        .await
        .unwrap();
    let second = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"second")
        .await
        .unwrap();
    let newest = engine
        .commit_upload(&actor, &root.id, &second)
        .await
        .unwrap();
    assert_ne!(newest.version_id, receipt.version_id);
    engine.tick().await.unwrap();
    let pending: i64 =
        sqlx::query_scalar("SELECT pending_cleanup FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    let usage = writer.root_usage(&actor, &root.id).await.unwrap();
    assert_eq!(usage.pending_cleanup_bytes.get(), pending as u64);
    assert!(usage.staging_occupied_bytes.get() >= usage.pending_cleanup_bytes.get());
    assert!(pending > 0);
    assert_eq!(
        engine
            .commit_upload(&actor, &root.id, &first)
            .await
            .unwrap(),
        receipt
    );
    assert!(
        writer
            .pin_version(&actor, &root.id, &receipt.version_id)
            .await
            .is_err()
    );
    writer.renew_read_pin(&actor, &root.id, &pin).await.unwrap();
    writer.release_read_pin(&actor, &pin).await.unwrap();
    engine.tick().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT occupied FROM allocations WHERE root_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let entry = writer
        .resolve_path(&actor, &root.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    let history = engine
        .file_history(&actor, &root.id, &entry.id, None)
        .await
        .unwrap();
    assert_eq!(history.versions.len(), 1);
    assert_eq!(history.versions[0].id, newest.version_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM transfer_audit WHERE event='publication'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        2
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn relaxed_admission_prunes_before_bytes_and_survives_later_abort() {
    use sct_core::{
        payload::{MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let mut request = request(actor.principal_id(), "/relaxed", 4000);
    request.quota_bytes = 10.try_into().unwrap();
    request.default_policy = RetentionPolicy {
        mode: RetentionMode::Relaxed,
        keep_versions: 3.try_into().unwrap(),
    };
    let root = writer.create_root(&actor, &request).await.unwrap();
    let root = writer
        .update_root(
            &actor,
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
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([(
            "one".into(),
            Arc::new(MemoryStore::default()) as Arc<dyn ObjectStore>,
        )]),
        2,
        86400,
    )
    .unwrap();
    let first = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"1111")
        .await
        .unwrap();
    let oldest = engine
        .commit_upload(&actor, &root.id, &first)
        .await
        .unwrap();
    let second = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"2222")
        .await
        .unwrap();
    let latest = engine
        .commit_upload(&actor, &root.id, &second)
        .await
        .unwrap();
    let pin = writer
        .pin_version(&actor, &root.id, &oldest.version_id)
        .await
        .unwrap();
    let blocked = plan_bytes(&engine, &writer, &actor, &root.id, "file", b"3333")
        .await
        .unwrap();
    let mut failed = false;
    for _ in 0..20 {
        if let Err(e) = engine.admit_upload(&actor, &root.id, &blocked).await {
            assert_eq!(e.0, ErrorCode::QuotaExhausted);
            failed = true;
            break;
        }
    }
    assert!(failed);
    engine
        .abort_upload(&actor, &root.id, &blocked)
        .await
        .unwrap();
    writer.release_read_pin(&actor, &pin).await.unwrap();
    let third = plan_bytes(&engine, &writer, &actor, &root.id, "file", b"3333")
        .await
        .unwrap();
    let mut admitted = false;
    for _ in 0..20 {
        if engine
            .admit_upload(&actor, &root.id, &third)
            .await
            .unwrap()
            .admission
            == AdmissionState::Admitted
        {
            admitted = true;
            break;
        }
    }
    assert!(admitted);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM transfer_audit WHERE event='admission_retirement'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let entry = writer
        .resolve_path(&actor, &root.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(entry.current_version_id, Some(latest.version_id.clone()));
    // No third payload has been uploaded, but old relaxed history was reclaimed.
    let history = engine
        .file_history(&actor, &root.id, &entry.id, None)
        .await
        .unwrap();
    assert_eq!(history.versions.len(), 1);
    assert_eq!(history.versions[0].id, latest.version_id);
    engine.abort_upload(&actor, &root.id, &third).await.unwrap();
    assert_eq!(
        engine
            .file_history(&actor, &root.id, &entry.id, None)
            .await
            .unwrap()
            .versions
            .len(),
        1
    );
    assert_eq!(
        engine
            .commit_upload(&actor, &root.id, &first)
            .await
            .unwrap(),
        oldest
    );
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL"]
async fn verified_download_ranges_fallback_and_pin_expiry() {
    use sct_core::{
        payload::{Fault, MemoryStore, ObjectStore},
        transfer_io::TransferEngine,
    };
    use std::collections::BTreeMap;
    let (writer, actor, pool) = setup().await;
    let root = root(&writer, &actor, "/download", 4000).await;
    let placement: Placement = serde_json::from_value(json!({"kind":"mirror","backend_ids":["one","two"],"read_policy":{"candidates":[{"backend_id":"one","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000},{"backend_id":"two","connection_timeout_ms":100,"initial_response_timeout_ms":100,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}})).unwrap();
    let root = writer
        .update_root(
            &actor,
            &root.id,
            &UpdateRootRequest {
                expected_revision: root.revision,
                display_name: root.display_name,
                state: RootState::Enabled,
                placement,
                quota_bytes: 2000.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    let one = Arc::new(MemoryStore::default());
    let two = Arc::new(MemoryStore::default());
    let engine = TransferEngine::new(
        writer.clone(),
        BTreeMap::from([
            ("one".into(), one.clone() as Arc<dyn ObjectStore>),
            ("two".into(), two.clone() as Arc<dyn ObjectStore>),
        ]),
        4,
        86400,
    )
    .unwrap();
    let upload = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"abcdef")
        .await
        .unwrap();
    let receipt = engine
        .commit_upload(&actor, &root.id, &upload)
        .await
        .unwrap();
    for fault in [Fault::CorruptRead, Fault::TruncateRead, Fault::Offline] {
        one.fault(fault).await;
        let mut read = engine
            .download(&actor, &root.id, &receipt.version_id, Some((1, 4)))
            .await
            .unwrap();
        assert_eq!(read.next().await.unwrap().unwrap(), b"bcd");
        assert!(read.next().await.unwrap().is_none());
    }
    two.fault(Fault::CorruptRead).await;
    let mut read = engine
        .download(&actor, &root.id, &receipt.version_id, None)
        .await
        .unwrap();
    assert!(read.next().await.is_err()); // No unverified bytes are released.
    drop(read);
    one.fault(Fault::None).await;
    two.fault(Fault::None).await;
    let mut pinned = engine
        .download(&actor, &root.id, &receipt.version_id, None)
        .await
        .unwrap();
    let replacement = ready_bytes(&engine, &writer, &actor, &root.id, "file", b"replacement")
        .await
        .unwrap();
    engine
        .commit_upload(&actor, &root.id, &replacement)
        .await
        .unwrap();
    engine.tick().await.unwrap();
    assert!(
        engine
            .download(&actor, &root.id, &receipt.version_id, None)
            .await
            .is_err()
    );
    assert_eq!(pinned.next().await.unwrap().unwrap(), b"abcdef");
    sqlx::query("UPDATE read_pins SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(pinned.next().await.is_err());
}

#[path = "support/purge_tests.rs"]
mod purge_tests;

#[path = "support/scale_tests.rs"]
mod scale_tests;
