mod support;
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
