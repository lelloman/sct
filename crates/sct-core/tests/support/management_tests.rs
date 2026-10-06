use super::*;
use simple_server::database::postgres as db;

async fn regular(
    w: &Writer,
    admin: &Actor,
    name: &str,
) -> (Actor, sct_core::auth::IssuedCredential) {
    let principal = name.parse().unwrap();
    w.create_principal(admin, &principal).await.unwrap();
    let credential = w.issue_token(admin, &principal).await.unwrap();
    let actor = w
        .authenticate(&credential.secret, false, None)
        .await
        .unwrap();
    (actor, credential)
}
fn page(limit: u64) -> ListRequest {
    ListRequest {
        limit: limit.try_into().unwrap(),
        cursor: None,
    }
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn delegated_management_enforces_ceiling_revisions_and_content_separation() {
    let (w, admin, db) = setup().await;
    let r = root(&w, &admin, "/managed", 4000).await;
    let (user, _) = regular(&w, &admin, "user").await;
    let grant = w
        .create_grant(
            &admin,
            &CreateGrantRequest {
                root_id: r.id.clone(),
                principal_id: "user".parse().unwrap(),
                capabilities: vec![
                    Capability::ListRead,
                    Capability::ManageRoot,
                    Capability::ManageGrants,
                ],
            },
        )
        .await
        .unwrap();
    let state = w.root_management(&user, &r.id).await.unwrap();
    assert_eq!(state.quota_ceiling_bytes.get(), 4000);
    assert!(!state.capabilities.contains(&Capability::Write));
    let mut request = UpdateRootManagementRequest {
        expected_revision: state.revision.clone(),
        display_name: "User root".parse().unwrap(),
        quota_bytes: 4001.try_into().unwrap(),
        expected_upload_interval_seconds: Some(3600.try_into().unwrap()),
    };
    assert_eq!(
        w.update_root_management(&user, &r.id, &request)
            .await
            .unwrap_err()
            .0,
        ErrorCode::QuotaExhausted
    );
    assert_eq!(
        w.update_quota_ceiling(
            &user,
            &r.id,
            &UpdateQuotaCeilingRequest {
                expected_revision: state.revision.clone(),
                quota_ceiling_bytes: 5000.try_into().unwrap()
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    request.quota_bytes = 3000.try_into().unwrap();
    let updated = w
        .update_root_management(&user, &r.id, &request)
        .await
        .unwrap();
    assert_eq!(updated.quota_bytes.get(), 3000);
    assert_eq!(updated.quota_ceiling_bytes.get(), 4000);
    assert_eq!(
        w.update_root_management(&user, &r.id, &request)
            .await
            .unwrap_err()
            .0,
        ErrorCode::RevisionConflict
    );
    assert_eq!(
        w.update_grant(
            &user,
            &grant.id,
            &UpdateGrantRequest {
                expected_revision: grant.revision.clone(),
                capabilities: vec![Capability::ManageRoot, Capability::Write]
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::Forbidden
    );
    let lower = w
        .update_quota_ceiling(
            &admin,
            &r.id,
            &UpdateQuotaCeilingRequest {
                expected_revision: updated.revision.clone(),
                quota_ceiling_bytes: 3500.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    request.expected_revision = lower.revision;
    request.quota_bytes = 3500.try_into().unwrap();
    let (a, b) = tokio::join!(
        w.update_root_management(&user, &r.id, &request),
        w.update_root_management(&user, &r.id, &request)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        db::query_scalar::<_, i64>(
            "SELECT count(*) FROM management_audit WHERE event='root.settings_updated'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        2
    );
    w.revoke_grant(&admin, &grant.id, &grant.revision)
        .await
        .unwrap();
    assert_eq!(
        w.root_management(&user, &r.id).await.unwrap_err().0,
        ErrorCode::NotFound
    );
    assert!(
        w.inspect_root(&admin, &r.id)
            .await
            .unwrap()
            .capabilities
            .contains(&Capability::Write)
    );
}

#[simple_server::test(host_runtime = true)]
#[ignore = "requires isolated PostgreSQL"]
async fn accounts_scope_cursors_hide_secrets_and_recheck_revocation() {
    let (w, admin, db) = setup().await;
    let (user, credential) = regular(&w, &admin, "alice").await;
    let (other, _) = regular(&w, &admin, "bob").await;
    let second = w
        .issue_token(&user, &"alice".parse().unwrap())
        .await
        .unwrap();
    let first = w
        .list_credentials(&user, &"alice".parse().unwrap(), &page(1))
        .await
        .unwrap();
    assert_eq!(first.credentials.len(), 1);
    let next = ListRequest {
        cursor: first.next_cursor.clone(),
        ..page(1)
    };
    assert!(next.cursor.is_some());
    let more = w
        .list_credentials(&user, &"alice".parse().unwrap(), &next)
        .await
        .unwrap();
    assert_eq!(more.credentials.len(), 1);
    assert_ne!(first.credentials[0].id, more.credentials[0].id);
    assert_eq!(
        w.list_credentials(&other, &"bob".parse().unwrap(), &next)
            .await
            .unwrap_err()
            .0,
        ErrorCode::CursorRefreshRequired
    );
    assert_eq!(
        w.list_credentials(&user, &"bob".parse().unwrap(), &page(1))
            .await
            .unwrap_err()
            .0,
        ErrorCode::Forbidden
    );
    assert_eq!(
        w.list_principals(&user, &page(1)).await.unwrap_err().0,
        ErrorCode::Forbidden
    );
    let output = serde_json::to_string(&first).unwrap();
    assert!(!output.contains("secret"));
    assert!(!output.contains(&credential.secret));
    assert!(!output.contains(&second.secret));
    w.revoke_credential(&user, &second.id).await.unwrap();
    assert!(w.authenticate(&second.secret, false, None).await.is_err());
    let current = w
        .inspect_principal(&admin, &"alice".parse().unwrap())
        .await
        .unwrap();
    w.update_principal(
        &admin,
        &current.id,
        &UpdatePrincipalRequest {
            expected_revision: current.revision,
            disabled: true,
            administrator: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        w.account(&user).await.unwrap_err().0,
        ErrorCode::Unauthorized
    );
    assert!(w.issue_token(&admin, &current.id).await.is_err());
    let admin_state = w
        .inspect_principal(&admin, &admin.principal_id().parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        w.update_principal(
            &admin,
            &admin_state.id,
            &UpdatePrincipalRequest {
                expected_revision: admin_state.revision,
                disabled: true,
                administrator: true
            }
        )
        .await
        .unwrap_err()
        .0,
        ErrorCode::InvalidRequest
    );
    let audit: String =
        db::query_scalar("SELECT coalesce(string_agg(details::text,','),'') FROM management_audit")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(!audit.contains(&credential.secret));
    assert!(!audit.contains(&second.secret));
}
