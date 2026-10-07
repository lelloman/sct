use super::policy_preview_tests::complete_preview;
use super::*;
use sct_core::{
    payload::{Fault, MemoryStore, ObjectStore},
    transfer_io::TransferEngine,
};
use std::collections::BTreeMap;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn policy_jobs_resume_forward_recheck_authority_and_wait_for_pinned_cleanup() {
    let (w, a, db) = setup().await;
    let mut req = request(a.principal_id(), "/jobs", 4000);
    req.default_policy.keep_versions = 200.try_into().unwrap();
    let r = w.create_root(&a, &req).await.unwrap();
    let r = w
        .update_root(
            &a,
            &r.id,
            &UpdateRootRequest {
                expected_revision: r.revision.clone(),
                display_name: r.display_name.clone(),
                state: RootState::Enabled,
                placement: r.placement.clone(),
                quota_bytes: r.quota_bytes,
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE pools SET capacity=220000")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE allocations SET capacity=100000")
        .execute(&db)
        .await
        .unwrap();
    let memory = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), memory.clone() as Arc<dyn ObjectStore>)]);
    let e = TransferEngine::new(w.clone(), stores.clone(), 2, 60).unwrap();
    let upload = ready_bytes(&e, &w, &a, &r.id, "file", b"old")
        .await
        .unwrap();
    let old = e.commit_upload(&a, &r.id, &upload).await.unwrap();
    let file = w
        .resolve_path(&a, &r.id, &"/file".parse().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO file_versions(id,root_id,entry_id,manifest,length) SELECT 'copy-'||n,v.root_id,v.entry_id,v.manifest,v.length FROM file_versions v CROSS JOIN generate_series(1,129) n WHERE v.id=$1")
        .bind(old.version_id.as_str()).execute(&db).await.unwrap();
    sqlx::query("UPDATE entries SET current_version_id=(SELECT id FROM file_versions WHERE entry_id=$1 ORDER BY sequence DESC LIMIT 1) WHERE id=$1").bind(file.id.as_str()).execute(&db).await.unwrap();
    let upload = ready_bytes(&e, &w, &a, &r.id, "file", b"new")
        .await
        .unwrap();
    let newest = e.commit_upload(&a, &r.id, &upload).await.unwrap();
    let pin = w.pin_version(&a, &r.id, &old.version_id).await.unwrap();
    w.create_principal(&a, &"operator".parse().unwrap())
        .await
        .unwrap();
    let token = w
        .issue_token(&a, &"operator".parse().unwrap())
        .await
        .unwrap();
    let operator = w.authenticate(&token.secret, false, None).await.unwrap();
    let grant = w
        .create_grant(
            &a,
            &CreateGrantRequest {
                root_id: r.id.clone(),
                principal_id: "operator".parse().unwrap(),
                capabilities: vec![Capability::ManagePolicy, Capability::ListRead],
            },
        )
        .await
        .unwrap();
    let scope = PolicyScope::Root {
        root_id: r.id.clone(),
    };
    let policy = w.effective_policy(&a, &scope).await.unwrap();
    let preview = complete_preview(
        &w,
        &operator,
        &r.id,
        PolicyChange::Set {
            scope: scope.clone(),
            expected_policy_revision: policy.revision,
            policy: RetentionPolicy {
                mode: RetentionMode::Strict,
                keep_versions: 1.try_into().unwrap(),
            },
        },
    )
    .await;
    assert_eq!(preview.versions_to_retire.get(), 130);
    let request = ApplyChangeRequest {
        idempotency_key: "apply".parse().unwrap(),
        preview_id: preview.id.clone(),
        expected_root_revision: preview.root_revision.clone(),
        acknowledge_retention_effect: true,
    };
    let mut refused = request.clone();
    refused.acknowledge_retention_effect = false;
    assert_eq!(
        e.apply_change(&operator, &r.id, &refused)
            .await
            .unwrap_err()
            .0,
        ErrorCode::InvalidRequest
    );
    let job = e.apply_change(&operator, &r.id, &request).await.unwrap();
    let activity = w
        .list_job_activity(
            &operator,
            Some(&r.id),
            &ListRequest {
                limit: 100.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        activity
            .jobs
            .iter()
            .find(|item| item.operation.id == job.id)
            .unwrap()
            .lifecycle
            .as_ref(),
        Some(&job)
    );
    assert_eq!(job.state, LifecycleState::Applying);
    assert_eq!(job.total.unwrap().get(), 130);
    assert_eq!(
        e.apply_change(&operator, &r.id, &request).await.unwrap().id,
        job.id
    );
    let effective = w.effective_policy(&a, &scope).await.unwrap();
    assert_eq!(effective.policy.keep_versions.get(), 1);
    assert!(effective.transitioning);
    assert_eq!(
        plan_bytes(&e, &w, &a, &r.id, "busy", b"x")
            .await
            .unwrap_err()
            .0,
        ErrorCode::RootBusy
    );
    let mut reader = e
        .download(&a, &r.id, &newest.version_id, None)
        .await
        .unwrap();
    assert_eq!(reader.next().await.unwrap().unwrap(), b"new");
    drop(reader);
    let partial = e.advance_lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    assert_eq!(partial.processed.get(), 64);
    w.update_grant(
        &a,
        &grant.id,
        &UpdateGrantRequest {
            expected_revision: grant.revision,
            capabilities: vec![Capability::ListRead],
        },
    )
    .await
    .unwrap();
    let blocked = e.advance_lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    assert_eq!(blocked.state, LifecycleState::Blocked);
    assert_eq!(blocked.failure, Some(ErrorCode::Forbidden));
    assert_eq!(blocked.processed.get(), 64);
    w.release().await.unwrap();
    let w = w.catalog().acquire_writer().await.unwrap();
    let e = TransferEngine::new(w.clone(), stores, 2, 60).unwrap();
    e.tick().await.unwrap();
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id)
            .await
            .unwrap()
            .processed
            .get(),
        64
    );
    assert!(
        e.resume_lifecycle_job(&operator, &r.id, &job.id)
            .await
            .is_err()
    );
    e.resume_lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    for _ in 0..12 {
        e.advance_lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    }
    let cleaning = e.lifecycle_job(&a, &r.id, &job.id).await.unwrap();
    assert_eq!(cleaning.state, LifecycleState::CleaningUp);
    assert_eq!(cleaning.processed.get(), 130);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM file_versions WHERE NOT retired")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        w.inspect_entry(&a, &file.id)
            .await
            .unwrap()
            .current_version_id,
        Some(newest.version_id)
    );
    w.renew_read_pin(&a, &r.id, &pin).await.unwrap();
    w.release_read_pin(&a, &pin).await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT sum(occupied)::bigint FROM allocations")
        .fetch_one(&db)
        .await
        .unwrap();
    memory.fault(Fault::UnknownDelete).await;
    e.tick().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT sum(occupied)::bigint FROM allocations")
            .fetch_one(&db)
            .await
            .unwrap(),
        before
    );
    assert_ne!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Completed
    );
    memory.fault(Fault::None).await;
    for _ in 0..12 {
        e.tick().await.unwrap();
        if e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state == LifecycleState::Completed {
            break;
        }
    }
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &job.id).await.unwrap().state,
        LifecycleState::Completed
    );
    assert!(!w.effective_policy(&a, &scope).await.unwrap().transitioning);
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT sum(occupied)::bigint FROM allocations")
            .fetch_one(&db)
            .await
            .unwrap()
            < before
    );
    // A stricter/larger policy becomes effective at apply; it never recreates
    // the historical versions already retired by the earlier job.
    let policy = w.effective_policy(&a, &scope).await.unwrap();
    let raised = complete_preview(
        &w,
        &a,
        &r.id,
        PolicyChange::Set {
            scope: scope.clone(),
            expected_policy_revision: policy.revision,
            policy: RetentionPolicy {
                mode: RetentionMode::Strict,
                keep_versions: 5.try_into().unwrap(),
            },
        },
    )
    .await;
    let raised_job = e
        .apply_change(
            &a,
            &r.id,
            &ApplyChangeRequest {
                idempotency_key: "raise".parse().unwrap(),
                preview_id: raised.id,
                expected_root_revision: raised.root_revision,
                acknowledge_retention_effect: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        w.effective_policy(&a, &scope)
            .await
            .unwrap()
            .policy
            .keep_versions
            .get(),
        5
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM file_versions WHERE NOT retired")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    for _ in 0..4 {
        e.tick().await.unwrap();
    }
    assert_eq!(
        e.lifecycle_job(&a, &r.id, &raised_job.id)
            .await
            .unwrap()
            .state,
        LifecycleState::Completed
    );
}
