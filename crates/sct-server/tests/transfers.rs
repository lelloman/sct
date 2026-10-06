#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
#[path = "support/tree_workflow.rs"]
mod tree_workflow;
use sct_client::{Client, UploadJob};
use sct_core::{
    backend::Registry,
    local_store::LocalAdapter,
    payload::{DeleteOutcome, IoFuture, ObjectKey, ObjectStore, Reader},
    roots::{BackendSetup, PoolSetup},
    s3_store::S3Adapter,
    transfer_io::TransferEngine,
};
use sct_protocol::*;
use serde_json::json;
use sqlx::Row;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

// Fault injection wraps actual disk/S3 I/O; acknowledged writes really reach the
// destination, then their response is lost. No payload mock replaces a backend.
struct LoseAck {
    inner: Arc<dyn ObjectStore>,
    lose: AtomicBool,
}
impl ObjectStore for LoseAck {
    fn put<'a>(&'a self, k: &'a ObjectKey, n: u64, b: Reader) -> IoFuture<'a, ()> {
        Box::pin(async move {
            self.inner.put(k, n, b).await?;
            if self.lose.swap(false, Ordering::SeqCst) {
                Err(sct_core::error::Error(ErrorCode::DestinationUnavailable))
            } else {
                Ok(())
            }
        })
    }
    fn read<'a>(&'a self, k: &'a ObjectKey) -> IoFuture<'a, Reader> {
        self.inner.read(k)
    }
    fn read_with_policy<'a>(
        &'a self,
        k: &'a ObjectKey,
        p: &'a ReadCandidate,
    ) -> IoFuture<'a, Reader> {
        self.inner.read_with_policy(k, p)
    }
    fn stat<'a>(&'a self, k: &'a ObjectKey) -> IoFuture<'a, Option<u64>> {
        self.inner.stat(k)
    }
    fn delete<'a>(&'a self, k: &'a ObjectKey) -> IoFuture<'a, DeleteOutcome> {
        self.inner.delete(k)
    }
    fn available(&self) -> IoFuture<'_, Option<u64>> {
        self.inner.available()
    }
}
async fn job(client: &Client, root: &RootId, source: &Path, directory: &Path) -> UploadJob {
    let parent = client
        .resolve_path(root, &"/".parse().unwrap())
        .await
        .unwrap();
    let target = match client.resolve_path(root, &"/file".parse().unwrap()).await {
        Ok(entry) => FileTarget::Replace {
            entry_id: entry.id,
            expected_revision: entry.revision,
        },
        Err(sct_client::Error::Server(e)) if e.code == ErrorCode::NotFound => FileTarget::Create {
            parent_id: parent.id,
            expected_parent_revision: parent.revision,
            name: "file".parse().unwrap(),
        },
        other => panic!("unexpected resolve: {other:?}"),
    };
    let policy = client
        .effective_policy(&PolicyScope::Root {
            root_id: root.clone(),
        })
        .await
        .unwrap();
    UploadJob::prepare(
        source,
        directory,
        target,
        policy.revision,
        uuid::Uuid::new_v4().simple().to_string().parse().unwrap(),
    )
    .unwrap()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL and S3; scripts/test-transfers"]
async fn real_transfer_placements() {
    let endpoint = std::env::var("SCT_TEST_S3_ENDPOINT").expect("real S3 required");
    let (catalog, pool) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&admin.secret, false, None)
        .await
        .unwrap();
    let directory =
        std::env::temp_dir().join(format!("sct-real-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&directory).unwrap();
    let mut registry = Registry::default();
    registry.register(Arc::new(LocalAdapter)).unwrap();
    registry.register(Arc::new(S3Adapter)).unwrap();
    let mut backends = Vec::new();
    let mut stores = BTreeMap::new();
    let mut injectors = BTreeMap::new();
    for (name, kind) in [
        ("local1", "local"),
        ("local2", "local"),
        ("s3one", "s3"),
        ("s3two", "s3"),
    ] {
        let settings = if kind == "local" {
            let path = directory.join(name);
            std::fs::create_dir(&path).unwrap();
            json!({"directory":path})
        } else {
            json!({"endpoint":endpoint,"bucket":format!("sct-{name}"),"region":"us-east-1","access_key_env":"SCT_TEST_S3_ACCESS","secret_key_env":"SCT_TEST_S3_SECRET"})
        };
        let adapter = registry.get(kind).unwrap();
        let store = Arc::new(LoseAck {
            inner: adapter.open(&settings).unwrap(),
            lose: AtomicBool::new(false),
        });
        store
            .available()
            .await
            .expect("configured real backend must be available");
        stores.insert(name.into(), store.clone() as Arc<dyn ObjectStore>);
        injectors.insert(name, store);
        backends.push(BackendSetup{pool_id:"pool".parse().unwrap(),inspection:serde_json::from_value(json!({"id":name,"display_name":name,"adapter_kind":kind,"hints":{"expected_initial_response_ms":null,"expected_bytes_per_second":null,"expected_availability":null},"observed_health":"unknown","last_observed_at_ms":null})).unwrap(),settings});
    }
    writer
        .configure(
            &registry,
            &[PoolSetup {
                id: "pool".parse().unwrap(),
                capacity: 8_000_000_000u64.try_into().unwrap(),
                staging: 128_000_000u64.try_into().unwrap(),
            }],
            &backends,
        )
        .await
        .unwrap();
    let engine = TransferEngine::new(writer.clone(), stores, 4, 86400).unwrap();
    let renewal_writer = writer.clone();
    let renewal_engine = engine.clone();
    let renewal = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            renewal_writer.renew().await.unwrap();
            renewal_engine.heartbeat().await.unwrap();
        }
    });
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
    let server = tokio::spawn(async move {
        sct_server::web::serve(
            listener,
            sct_server::application(app, "nonexistent".into()),
            simple_server::lifecycle::Shutdown::new(),
        )
        .await
        .unwrap()
    });
    let client = Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin.secret.clone())
        .unwrap();
    let mut state = 0x9e3779b97f4a7c15u64;
    let bytes: Vec<u8> = (0..6 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    for (index, names) in [
        vec!["local1"],
        vec!["s3one"],
        vec!["local1", "s3one"],
        vec!["local1", "local2"],
        vec!["s3one", "s3two"],
    ]
    .into_iter()
    .enumerate()
    {
        let candidates:Vec<_>=names.iter().map(|name|json!({"backend_id":name,"connection_timeout_ms":1000,"initial_response_timeout_ms":1000,"idle_timeout_ms":5000,"throughput_window_ms":5000,"minimum_bytes_per_second":1,"startup_allowance_ms":1000})).collect();
        let policy = json!({"candidates":candidates,"overall_deadline_ms":null});
        let placement = if names.len() == 1 {
            json!({"kind":"single","backend_id":names[0],"read_policy":policy})
        } else {
            json!({"kind":"mirror","backend_ids":names,"read_policy":policy})
        };
        let request:CreateRootRequest=serde_json::from_value(json!({"idempotency_key":format!("root-{index}"),"owner_id":actor.principal_id(),"display_name":"Real transfer","mount_path":format!("/real-{index}"),"placement":placement,"quota_bytes":100_000_000,"default_policy":{"mode":"strict","keep_versions":1},"allocations":[{"pool_id":"pool","retained_capacity_bytes":500_000_000}]})).unwrap();
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
        let source = directory.join(format!("source-{index}"));
        std::fs::write(&source, &bytes).unwrap();
        let job_path = directory.join(format!("job-{index}"));
        let plan = job(&client, &root.id, &source, &job_path).await;
        // First attempt physically writes a copy, loses its response, then resumes
        // from only the saved job. Server recovery verifies the uncertain bytes.
        injectors[names.last().unwrap()]
            .lose
            .store(true, Ordering::SeqCst);
        let failure = client.upload(&root.id, &plan).await.unwrap_err();
        assert!(
            matches!(failure,sct_client::Error::Server(ref e) if e.code==ErrorCode::DestinationUnavailable),
            "unexpected injected error: {failure:?}"
        );
        engine.tick().await.unwrap();
        drop(plan);
        let receipt = client
            .upload(&root.id, &UploadJob::open(&job_path).unwrap())
            .await
            .unwrap();
        assert_eq!(
            client
                .upload(&root.id, &UploadJob::open(&job_path).unwrap())
                .await
                .unwrap(),
            receipt
        );
        let mut output = Vec::new();
        client
            .download_to(&root.id, &receipt.version_id, None, &mut output)
            .await
            .unwrap();
        assert_eq!(output, bytes);
        output.clear();
        client
            .download_to(
                &root.id,
                &receipt.version_id,
                Some((250_000, 2_000_000)),
                &mut output,
            )
            .await
            .unwrap();
        assert_eq!(output, bytes[250_000..2_000_000]);
        // Replace while a reader owns a pin, keeping old bytes through cleanup.
        let mut pinned = engine
            .download(&actor, &root.id, &receipt.version_id, None)
            .await
            .unwrap();
        std::fs::write(&source, b"replacement").unwrap();
        let replacement = job(
            &client,
            &root.id,
            &source,
            &directory.join(format!("replacement-{index}")),
        )
        .await;
        let latest = client.upload(&root.id, &replacement).await.unwrap();
        engine.tick().await.unwrap();
        let mut old = Vec::new();
        while let Some(chunk) = pinned.next().await.unwrap() {
            old.extend(chunk)
        }
        assert_eq!(old, bytes);
        drop(pinned);
        tokio::task::yield_now().await;
        engine.tick().await.unwrap();
        let entry = client
            .resolve_path(&root.id, &"/file".parse().unwrap())
            .await
            .unwrap();
        let history = client
            .file_history(&root.id, &entry.id, None)
            .await
            .unwrap();
        assert_eq!(history.versions.len(), 1);
        assert_eq!(history.versions[0].id, latest.version_id);
        // Ensure both physical copies are charged even in the same pool.
        let ledger: i64 = sqlx::query_scalar("SELECT occupied FROM allocations WHERE root_id=$1")
            .bind(root.id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
        let physical:i64=sqlx::query_scalar("SELECT COALESCE(sum(c.bytes),0)::bigint FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE a.root_id=$1 AND c.state='available'").bind(root.id.as_str()).fetch_one(&pool).await.unwrap();
        assert_eq!(ledger, physical);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM object_copies WHERE root_id=$1 AND digest=$2 AND state='available'").bind(root.id.as_str()).bind(digest(b"replacement")).fetch_one(&pool).await.unwrap(),names.len() as i64);
        // Changed reusable/source bytes must not publish a plan silently.
        let changed = job(
            &client,
            &root.id,
            &source,
            &directory.join(format!("changed-{index}")),
        )
        .await;
        std::fs::write(&source, b"changed").unwrap();
        assert!(matches!(
            client.upload(&root.id, &changed).await,
            Err(sct_client::Error::SourceChanged)
        ));
        let status = client
            .create_upload(&root.id, changed.request())
            .await
            .unwrap();
        client
            .abort_upload(&root.id, &status.upload.id)
            .await
            .unwrap();
        engine.tick().await.unwrap();
        // Corrupt real local payload: a mirror succeeds; a single copy fails.
        if names[0] == "local1" {
            let row=sqlx::query("SELECT generation FROM object_copies WHERE root_id=$1 AND backend_id='local1' AND digest=$2 AND state='available'").bind(root.id.as_str()).bind(digest(b"replacement")).fetch_one(&pool).await.unwrap();
            let key = ObjectKey {
                root: root.id.clone(),
                digest: digest(b"replacement").parse().unwrap(),
                generation: row.get::<String, _>("generation").parse().unwrap(),
            };
            std::fs::write(directory.join("local1").join(key.path()), b"bad").unwrap();
            let result = client
                .download_to(&root.id, &latest.version_id, None, &mut Vec::new())
                .await;
            assert_eq!(result.is_ok(), names.len() > 1);
        }

        if std::env::var_os("SCT_QUALIFY_TREES").is_some() {
            let mut tree_request = request.clone();
            tree_request.idempotency_key = format!("tree-{index}").parse().unwrap();
            tree_request.mount_path = format!("/tree-{index}").parse().unwrap();
            tree_request.default_policy.keep_versions = 3.try_into().unwrap();
            let tree = client.create_root(&tree_request).await.unwrap();
            let tree = client
                .update_root(
                    &tree.id,
                    &UpdateRootRequest {
                        expected_revision: tree.revision,
                        display_name: tree.display_name,
                        state: RootState::Enabled,
                        placement: tree.placement,
                        quota_bytes: tree.quota_bytes,
                    },
                )
                .await
                .unwrap();
            // Whole-tree quota rejection occurs before any physical copy exists.
            let parent = client
                .resolve_path(&tree.id, &"/".parse().unwrap())
                .await
                .unwrap();
            let capacity = client
                .create_directory(
                    &tree.id,
                    &CreateDirectoryRequest {
                        idempotency_key: format!("capacity-{index}").parse().unwrap(),
                        parent_id: parent.id,
                        expected_parent_revision: parent.revision,
                        name: "capacity".parse().unwrap(),
                        versioning: VersioningMode::AtomicDirectory,
                    },
                )
                .await
                .unwrap();
            let current = client.inspect_root(&tree.id).await.unwrap();
            client
                .update_root(
                    &tree.id,
                    &UpdateRootRequest {
                        expected_revision: current.revision,
                        display_name: current.display_name,
                        state: RootState::Enabled,
                        placement: current.placement,
                        quota_bytes: 1.try_into().unwrap(),
                    },
                )
                .await
                .unwrap();
            let policy = client
                .effective_policy(&PolicyScope::Entry {
                    root_id: tree.id.clone(),
                    entry_id: capacity.id.clone(),
                })
                .await
                .unwrap();
            let request = CreateTreeDraftRequest {
                idempotency_key: format!("capacity-draft-{index}").parse().unwrap(),
                base: DirectoryDraftBase {
                    root_id: tree.id.clone(),
                    directory_id: capacity.id.clone(),
                    expected_revision: capacity.revision,
                    base_version_id: None,
                    mode: DraftMode::CompleteReplacement,
                },
                expected_policy_revision: policy.revision,
            };
            let draft = client.create_tree_draft(&tree.id, &request).await.unwrap();
            let ids = client
                .allocate_tree_identities(
                    &tree.id,
                    &draft.id,
                    &AllocateDraftIdentitiesRequest {
                        idempotency_key: "capacity-ids".parse().unwrap(),
                        count: 1.try_into().unwrap(),
                    },
                )
                .await
                .unwrap();
            let plan = sct_client::TreeJob::prepare(
                &directory.join(format!("capacity-{index}")),
                request,
                [Ok(vec![sct_client::TreeMutation::File {
                    identity: DraftEntryIdentity::New {
                        entry_id: ids.entry_ids[0].clone(),
                    },
                    parent_id: capacity.id.clone(),
                    name: "file".parse().unwrap(),
                    source: source.clone(),
                }])],
            )
            .unwrap();
            assert!(matches!(client.upload_tree(&plan).await,
                Err(sct_client::Error::Server(ref error)) if error.code==ErrorCode::QuotaExhausted));
            assert!(
                client
                    .current_snapshot(&tree.id, &capacity.id)
                    .await
                    .is_err()
            );
            let copies: i64 =
                sqlx::query_scalar("SELECT count(*) FROM object_copies WHERE root_id=$1")
                    .bind(tree.id.as_str())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(copies, 0);
            for _ in 0..10 {
                engine.tick().await.unwrap();
            }
            let current = client.inspect_root(&tree.id).await.unwrap();
            client
                .update_root(
                    &tree.id,
                    &UpdateRootRequest {
                        expected_revision: current.revision,
                        display_name: current.display_name,
                        state: RootState::Enabled,
                        placement: current.placement,
                        quota_bytes: tree.quota_bytes,
                    },
                )
                .await
                .unwrap();
            tree_workflow::run(&client, &tree.id, &directory.join(format!("tree-{index}"))).await;
            println!("M3 tree lifecycle passed for {:?}", names);
        }

        let mut small_request = request.clone();
        small_request.idempotency_key = format!("small-{index}").parse().unwrap();
        small_request.mount_path = format!("/small-{index}").parse().unwrap();
        small_request.quota_bytes = 10.try_into().unwrap();
        small_request.default_policy = RetentionPolicy {
            mode: RetentionMode::Relaxed,
            keep_versions: 3.try_into().unwrap(),
        };
        let small = client.create_root(&small_request).await.unwrap();
        let small = client
            .update_root(
                &small.id,
                &UpdateRootRequest {
                    expected_revision: small.revision,
                    display_name: small.display_name,
                    state: RootState::Enabled,
                    placement: small.placement,
                    quota_bytes: small.quota_bytes,
                },
            )
            .await
            .unwrap();
        for (n, data) in [b"1111", b"2222"].iter().enumerate() {
            std::fs::write(&source, data).unwrap();
            let plan = job(
                &client,
                &small.id,
                &source,
                &directory.join(format!("small-{index}-{n}")),
            )
            .await;
            client.upload(&small.id, &plan).await.unwrap();
        }
        std::fs::write(&source, b"3333").unwrap();
        let plan = job(
            &client,
            &small.id,
            &source,
            &directory.join(format!("prune-{index}")),
        )
        .await;
        let upload = client
            .create_upload(&small.id, plan.request())
            .await
            .unwrap()
            .upload
            .id;
        for entry in std::fs::read_dir(directory.join(format!("prune-{index}/pages"))).unwrap() {
            client
                .upload_page(
                    &small.id,
                    &upload,
                    std::fs::read(entry.unwrap().path()).unwrap(),
                )
                .await
                .unwrap();
        }
        while client
            .admit_upload(&small.id, &upload)
            .await
            .unwrap()
            .admission
            != AdmissionState::Admitted
        {}
        let entry = client
            .resolve_path(&small.id, &"/file".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(
            client
                .file_history(&small.id, &entry.id, None)
                .await
                .unwrap()
                .versions
                .len(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM object_copies WHERE root_id=$1 AND digest=$2"
            )
            .bind(small.id.as_str())
            .bind(digest(b"3333"))
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        sqlx::query("UPDATE transfer_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(upload.as_str()).execute(&pool).await.unwrap();
        engine.tick().await.unwrap();
        assert_eq!(
            client
                .upload_status(&small.id, &upload)
                .await
                .unwrap()
                .upload
                .state,
            SessionState::Expired
        );
        assert_eq!(
            client
                .file_history(&small.id, &entry.id, None)
                .await
                .unwrap()
                .versions
                .len(),
            1
        );
        std::fs::write(&source, b"too-large!!!").unwrap();
        let plan = job(
            &client,
            &small.id,
            &source,
            &directory.join(format!("oversize-{index}")),
        )
        .await;
        let failure = client.upload(&small.id, &plan).await.unwrap_err();
        assert!(
            matches!(failure,sct_client::Error::Server(ref e) if e.code==ErrorCode::QuotaExhausted),
            "{failure:?}"
        );
        engine.tick().await.unwrap();
        let rejected = client
            .create_upload(&small.id, plan.request())
            .await
            .unwrap();
        assert_eq!(rejected.admission, AdmissionState::Rejected);
        assert_eq!(
            client
                .abort_upload(&small.id, &rejected.upload.id)
                .await
                .unwrap()
                .upload
                .state,
            SessionState::Aborted
        );
        assert_eq!(
            client
                .file_history(&small.id, &entry.id, None)
                .await
                .unwrap()
                .versions
                .len(),
            1
        );
        eprintln!(
            "real placement {names:?}: upload, lost-response resume, receipt replay, range, pinned replacement, accounting, source change, corruption passed"
        );
    }
    server.abort();
    renewal.abort();
    writer.release().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
