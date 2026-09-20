#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_client::{CheckpointArtifact, Client, TreeJob, TreeMutation, UploadJob, protocol::*};
use sct_core::{backend::Registry, local_store::LocalAdapter, transfer_io::TransferEngine};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tokio::io::AsyncWriteExt;
fn key() -> IdempotencyKey {
    uuid::Uuid::new_v4().to_string().parse().unwrap()
}
fn private(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}
async fn command(args: &[&str], recovery_url: Option<&str>) -> std::process::Output {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let recovery_url = recovery_url.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_sct-server"));
        command
            .args(args)
            .env("SCT_CONFIG", "/nonexistent-production-config");
        if let Some(url) = recovery_url {
            command.env("SCT_M4_RECOVERY_DATABASE_URL", url);
        }
        command.output().unwrap()
    })
    .await
    .unwrap()
}
fn success(output: std::process::Output) -> CheckpointDescriptor {
    assert!(
        output.status.success(),
        "operator command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn m4_independent_recovery_from_http_export_survives_loss_of_original_storage() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let token = writer.recover_administrator().await.unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    writer.configure(&registry, &[], &[]).await.unwrap();
    let directory = std::env::temp_dir().join(format!("sct-m4-recovery-{}", uuid::Uuid::new_v4()));
    let original = directory.join("original");
    let package = directory.join("package");
    let independent = directory.join("independent");
    let recovered = directory.join("recovered");
    for path in [&original, &package, &independent, &recovered] {
        private(path);
    }
    let engine = TransferEngine::new(writer.clone(), Default::default(), 4, 3600)
        .unwrap()
        .with_checkpoint_directory(directory.join("exports"));
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
        simple_server::axum::serve(listener, sct_server::application(app, "nonexistent".into()))
            .await
            .unwrap();
    });
    let client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(token.secret.clone())
        .unwrap();
    let account = client.account().await.unwrap();
    client
        .create_pool(&CreatePoolRequest {
            id: "m4-pool".parse().unwrap(),
            capacity_bytes: 2000000.try_into().unwrap(),
            staging_bytes: 500000.try_into().unwrap(),
        })
        .await
        .unwrap();
    let backend:ConfigureBackendRequest=serde_json::from_value(json!({"expected_revision":null,"pool_id":"m4-pool","inspection":{"id":"m4-local","display_name":"Original storage","adapter_kind":"local","hints":{},"observed_health":"unknown"},"settings":{"directory":original}})).unwrap();
    client
        .configure_backend(&backend.inspection.id, &backend)
        .await
        .unwrap();
    let request:CreateRootRequest=serde_json::from_value(json!({"idempotency_key":"m4-root","owner_id":account.principal_id,"display_name":"M4 recovery","mount_path":"/","placement":{"kind":"single","backend_id":"m4-local","read_policy":{"candidates":[{"backend_id":"m4-local","connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":1000,"throughput_window_ms":1000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000}],"overall_deadline_ms":null}},"quota_bytes":100000,"default_policy":{"mode":"strict","keep_versions":2},"allocations":[{"pool_id":"m4-pool","retained_capacity_bytes":1000000}]})).unwrap();
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
    let source = directory.join("captured-file");
    std::fs::write(&source, b"independent file backup").unwrap();
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
    let file_job = UploadJob::prepare(
        &source,
        &directory.join("file-job"),
        FileTarget::Create {
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name: "saved.txt".parse().unwrap(),
        },
        policy.revision,
        key(),
    )
    .unwrap();
    let file = client.upload(&root.id, &file_job).await.unwrap();
    engine.tick().await.unwrap();
    let parent = client
        .resolve_path(&root.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let atomic = client
        .create_directory(
            &root.id,
            &CreateDirectoryRequest {
                idempotency_key: key(),
                parent_id: parent.id,
                expected_parent_revision: parent.revision,
                name: "tree".parse().unwrap(),
                versioning: VersioningMode::AtomicDirectory,
            },
        )
        .await
        .unwrap();
    let policy = client
        .effective_policy(&PolicyScope::Entry {
            root_id: root.id.clone(),
            entry_id: atomic.id.clone(),
        })
        .await
        .unwrap();
    let draft = CreateTreeDraftRequest {
        idempotency_key: key(),
        base: DirectoryDraftBase {
            root_id: root.id.clone(),
            directory_id: atomic.id.clone(),
            expected_revision: atomic.revision,
            base_version_id: None,
            mode: DraftMode::CompleteReplacement,
        },
        expected_policy_revision: policy.revision,
    };
    let status = client.create_tree_draft(&root.id, &draft).await.unwrap();
    let ids = client
        .allocate_tree_identities(
            &root.id,
            &status.id,
            &AllocateDraftIdentitiesRequest {
                idempotency_key: key(),
                count: 1.try_into().unwrap(),
            },
        )
        .await
        .unwrap();
    let tree_source = directory.join("captured-tree-file");
    std::fs::write(&tree_source, b"independent tree backup").unwrap();
    let tree_job = TreeJob::prepare(
        &directory.join("tree-job"),
        draft,
        [Ok(vec![TreeMutation::File {
            identity: DraftEntryIdentity::New {
                entry_id: ids.entry_ids[0].clone(),
            },
            parent_id: atomic.id.clone(),
            name: "inside.txt".parse().unwrap(),
            source: tree_source,
        }])],
    )
    .unwrap();
    let tree = client.upload_tree(&tree_job).await.unwrap();
    engine.tick().await.unwrap();
    // Include an unfinished session: recovery must not make it visible as saved content.
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
    let unfinished = UploadJob::prepare(
        &source,
        &directory.join("unfinished-job"),
        FileTarget::Create {
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name: "unfinished.txt".parse().unwrap(),
        },
        policy.revision,
        key(),
    )
    .unwrap();
    let unfinished = client
        .create_upload(&root.id, unfinished.request())
        .await
        .unwrap()
        .upload
        .id;
    let checkpoint = client
        .create_checkpoint(&CreateCheckpointRequest {
            idempotency_key: key(),
        })
        .await
        .unwrap();
    for _ in 0..16 {
        engine.tick().await.unwrap();
        if client.checkpoint(&checkpoint.id).await.unwrap().state == CheckpointState::Exported {
            break;
        }
    }
    assert_eq!(
        client.checkpoint(&checkpoint.id).await.unwrap().state,
        CheckpointState::Exported
    );
    for (artifact, name) in [
        (CheckpointArtifact::Catalog, "catalog.jsonl"),
        (CheckpointArtifact::Inventory, "inventory.jsonl"),
        (CheckpointArtifact::Descriptor, "descriptor.json"),
    ] {
        let mut output = tokio::fs::File::create(package.join(name)).await.unwrap();
        client
            .download_checkpoint_artifact_to(&checkpoint.id, artifact, &mut output)
            .await
            .unwrap();
        output.flush().await.unwrap();
        output.sync_all().await.unwrap();
    }
    let sources = directory.join("sources.json");
    std::fs::write(
        &sources,
        serde_json::to_vec(&json!({"m4-local":{"kind":"local","settings":{"directory":original}}}))
            .unwrap(),
    )
    .unwrap();
    let package_str = package.to_str().unwrap();
    let independent_str = independent.to_str().unwrap();
    assert_eq!(
        success(
            command(
                &[
                    "copy-checkpoint-payloads",
                    package_str,
                    sources.to_str().unwrap(),
                    independent_str,
                    "--acknowledge-independent"
                ],
                None
            )
            .await
        )
        .package_state,
        RecoveryPackageState::PayloadVerified
    );
    server.abort();
    let _ = server.await;
    writer.release().await.unwrap();
    catalog.close().await;
    std::fs::remove_dir_all(&original).unwrap();
    assert!(!original.exists());
    assert_eq!(
        success(command(&["verify-checkpoint", package_str, independent_str], None).await)
            .package_state,
        RecoveryPackageState::PayloadVerified
    );
    let (target, db) = support::database().await;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&db)
        .await
        .unwrap();
    let database_url = std::env::var("SCT_TEST_DATABASE_URL").unwrap();
    let separator = if database_url.contains('?') { '&' } else { '?' };
    let target_url = format!("{database_url}{separator}options=-csearch_path%3D{schema}");
    let recovered_str = recovered.to_str().unwrap();
    assert!(
        !command(
            &[
                "recover-checkpoint",
                package_str,
                independent_str,
                recovered_str,
                "--database-url-env",
                "SCT_M4_RECOVERY_DATABASE_URL"
            ],
            Some(&target_url)
        )
        .await
        .status
        .success()
    );
    assert_eq!(
        success(
            command(
                &[
                    "recover-checkpoint",
                    package_str,
                    independent_str,
                    recovered_str,
                    "--database-url-env",
                    "SCT_M4_RECOVERY_DATABASE_URL",
                    "--acknowledge-isolated"
                ],
                Some(&target_url)
            )
            .await
        )
        .package_state,
        RecoveryPackageState::PayloadVerified
    );
    let restored = target.acquire_writer().await.unwrap();
    assert!(
        restored
            .authenticate(&token.secret, false, None)
            .await
            .is_err()
    );
    let recovery_token = restored.recover_administrator().await.unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    let mut stores = BTreeMap::new();
    for backend in restored.configured_backends().await.unwrap() {
        assert_ne!(backend.settings["directory"], json!(original));
        stores.insert(
            backend.inspection.id.as_str().to_owned(),
            registry
                .get(&backend.inspection.adapter_kind)
                .unwrap()
                .open(&backend.settings)
                .unwrap(),
        );
    }
    let engine = TransferEngine::new(restored.clone(), stores, 4, 3600).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: restored.clone(),
        registry,
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine),
    };
    let recovered_server = tokio::spawn(async move {
        simple_server::axum::serve(listener, sct_server::application(app, "nonexistent".into()))
            .await
            .unwrap();
    });
    let recovered_client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(recovery_token.secret)
        .unwrap();
    let mut bytes = Vec::new();
    recovered_client
        .download_to(&root.id, &file.version_id, None, &mut bytes)
        .await
        .unwrap();
    assert_eq!(bytes, b"independent file backup");
    bytes.clear();
    recovered_client
        .download_snapshot_to(
            &root.id,
            &tree.version_id,
            &"/inside.txt".parse().unwrap(),
            None,
            &mut bytes,
        )
        .await
        .unwrap();
    assert_eq!(bytes, b"independent tree backup");
    assert_eq!(
        recovered_client
            .upload_status(&root.id, &unfinished)
            .await
            .unwrap()
            .upload
            .state,
        SessionState::Expired
    );
    assert!(
        matches!(recovered_client.resolve_path(&root.id,&"/unfinished.txt".parse().unwrap()).await,Err(sct_client::Error::Server(e)) if e.code==ErrorCode::NotFound)
    );
    recovered_server.abort();
    let _ = recovered_server.await;
    restored.release().await.unwrap();
    target.close().await;
    db.close().await;
    std::fs::remove_dir_all(directory).unwrap();
    println!(
        "M4 independent recovery qualified: file and atomic-tree bytes recovered with original storage removed"
    );
}
