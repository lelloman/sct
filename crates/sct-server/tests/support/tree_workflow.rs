// This workflow deliberately uses only the public standalone SDK.
use sct_client::{Client, TreeJob, TreeMutation, protocol::*};
use std::path::Path;
fn key() -> IdempotencyKey {
    uuid::Uuid::new_v4().to_string().parse().unwrap()
}
async fn draft(
    client: &Client,
    root: &RootId,
    entry: &EntryId,
    mode: DraftMode,
) -> CreateTreeDraftRequest {
    let current = client.inspect_entry(entry).await.unwrap();
    let policy = client
        .effective_policy(&PolicyScope::Entry {
            root_id: root.clone(),
            entry_id: entry.clone(),
        })
        .await
        .unwrap();
    CreateTreeDraftRequest {
        idempotency_key: key(),
        base: DirectoryDraftBase {
            root_id: root.clone(),
            directory_id: entry.clone(),
            expected_revision: current.revision,
            base_version_id: current.current_version_id,
            mode,
        },
        expected_policy_revision: policy.revision,
    }
}
pub async fn run(client: &Client, root: &RootId, directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    let parent = client
        .resolve_path(root, &"/".parse().unwrap())
        .await
        .unwrap();
    let atomic = client
        .create_directory(
            root,
            &CreateDirectoryRequest {
                idempotency_key: key(),
                parent_id: parent.id,
                expected_parent_revision: parent.revision,
                name: "snapshot".parse().unwrap(),
                versioning: VersioningMode::AtomicDirectory,
            },
        )
        .await
        .unwrap();
    let source = directory.join("source");
    std::fs::write(&source, b"snapshot payload").unwrap();
    let request = draft(client, root, &atomic.id, DraftMode::CompleteReplacement).await;
    let status = client.create_tree_draft(root, &request).await.unwrap();
    let allocation = AllocateDraftIdentitiesRequest {
        idempotency_key: key(),
        count: 3.try_into().unwrap(),
    };
    let ids = client
        .allocate_tree_identities(root, &status.id, &allocation)
        .await
        .unwrap()
        .entry_ids;
    assert_eq!(
        client
            .allocate_tree_identities(root, &status.id, &allocation)
            .await
            .unwrap()
            .entry_ids,
        ids
    );
    let job_dir = directory.join("replacement");
    let _job = TreeJob::prepare(
        &job_dir,
        request,
        [
            Ok(vec![TreeMutation::Directory {
                identity: DraftEntryIdentity::New {
                    entry_id: ids[0].clone(),
                },
                parent_id: atomic.id.clone(),
                name: "nested".parse().unwrap(),
            }]),
            Ok(vec![
                TreeMutation::File {
                    identity: DraftEntryIdentity::New {
                        entry_id: ids[1].clone(),
                    },
                    parent_id: ids[0].clone(),
                    name: "payload".parse().unwrap(),
                    source: source.clone(),
                },
                TreeMutation::Directory {
                    identity: DraftEntryIdentity::New {
                        entry_id: ids[2].clone(),
                    },
                    parent_id: atomic.id.clone(),
                    name: "empty".parse().unwrap(),
                },
            ]),
        ],
    )
    .unwrap();
    // The first mutation reached the service, but the client disappeared before freezing.
    let first: DraftMutationPage =
        serde_json::from_slice(&std::fs::read(job_dir.join("mutations/0")).unwrap()).unwrap();
    client
        .mutate_tree_draft(root, &status.id, &first)
        .await
        .unwrap();
    std::fs::write(&source, b"changed source").unwrap();
    assert!(matches!(
        client.upload_tree(&TreeJob::open(&job_dir).unwrap()).await,
        Err(sct_client::Error::SourceChanged)
    ));
    std::fs::write(&source, b"snapshot payload").unwrap();
    let receipt = client
        .upload_tree(&TreeJob::open(&job_dir).unwrap())
        .await
        .unwrap();
    assert_eq!(
        client
            .upload_tree(&TreeJob::open(&job_dir).unwrap())
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        client.current_snapshot(root, &atomic.id).await.unwrap(),
        receipt.version_id
    );
    let query = SnapshotListQuery {
        path: "/".parse().unwrap(),
        limit: 1.try_into().unwrap(),
        cursor: None,
    };
    let first = client
        .snapshot_entries(root, &receipt.version_id, &query)
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 1);
    let second = client
        .snapshot_entries(
            root,
            &receipt.version_id,
            &SnapshotListQuery {
                cursor: first.next_cursor,
                ..query
            },
        )
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    let path: RootPath = "/nested/payload".parse().unwrap();
    assert_eq!(
        client
            .snapshot_entry(root, &receipt.version_id, &path)
            .await
            .unwrap()
            .id,
        ids[1]
    );
    let mut output = Vec::new();
    client
        .download_snapshot_to(root, &receipt.version_id, &path, Some((1, 8)), &mut output)
        .await
        .unwrap();
    assert_eq!(output, b"napshot");

    let patch_request = draft(client, root, &atomic.id, DraftMode::Patch).await;
    let patch_dir = directory.join("patch");
    let patch = TreeJob::prepare(
        &patch_dir,
        patch_request,
        [Ok(vec![TreeMutation::Remove {
            entry_id: ids[1].clone(),
        }])],
    )
    .unwrap();
    let patched = client.upload_tree(&patch).await.unwrap();
    assert_ne!(patched.version_id, receipt.version_id);
    assert!(
        client
            .snapshot_entry(root, &patched.version_id, &path)
            .await
            .is_err()
    );
    assert!(
        client
            .snapshot_entry(root, &receipt.version_id, &path)
            .await
            .is_ok()
    );

    let scope = PolicyScope::Entry {
        root_id: root.clone(),
        entry_id: atomic.id.clone(),
    };
    let policy = client.effective_policy(&scope).await.unwrap();
    let revision = client.inspect_root(root).await.unwrap().revision;
    let mut preview = client
        .preview_change(
            root,
            &PreviewChangeRequest {
                expected_root_revision: revision,
                change: PolicyChange::Set {
                    scope,
                    expected_policy_revision: policy.revision,
                    policy: RetentionPolicy {
                        mode: RetentionMode::Strict,
                        keep_versions: 1.try_into().unwrap(),
                    },
                },
            },
        )
        .await
        .unwrap();
    while preview.state == LifecycleState::Planning {
        preview = client.advance_preview(root, &preview.id).await.unwrap();
    }
    let effects = client
        .preview_versions(
            root,
            &preview.id,
            &ListRequest {
                limit: 1.try_into().unwrap(),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(effects.versions.len(), 1);
    let apply = ApplyChangeRequest {
        idempotency_key: key(),
        preview_id: preview.id,
        expected_root_revision: preview.root_revision,
        acknowledge_retention_effect: true,
    };
    let job = client.apply_change(root, &apply).await.unwrap();
    assert_eq!(client.apply_change(root, &apply).await.unwrap().id, job.id);
    finish(client, root, &job.id).await;
    assert!(
        client
            .snapshot_entry(root, &receipt.version_id, &path)
            .await
            .is_err()
    );
    let current = client.inspect_entry(&atomic.id).await.unwrap();
    let hidden = client
        .hide_entry(
            root,
            &atomic.id,
            &HideEntryRequest {
                idempotency_key: key(),
                expected_revision: current.revision,
            },
        )
        .await
        .unwrap();
    let request = PurgeRequest {
        idempotency_key: key(),
        expected_revision: hidden.revision,
        target: PurgeTarget::HiddenEntry {
            entry_id: atomic.id.clone(),
        },
    };
    let purge = client.purge(root, &request).await.unwrap();
    finish(client, root, &purge.id).await;
    assert_eq!(client.purge(root, &request).await.unwrap().id, purge.id);
    assert!(client.inspect_entry(&atomic.id).await.is_err());
}
async fn finish(client: &Client, root: &RootId, id: &OperationId) {
    for _ in 0..200 {
        let job = client.advance_lifecycle_job(root, id).await.unwrap();
        if job.state == LifecycleState::Completed {
            return;
        }
        assert_ne!(job.state, LifecycleState::Blocked, "{:?}", job.failure);
    }
    panic!("job did not finish")
}
