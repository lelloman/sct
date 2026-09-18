use sct_protocol::*;
use serde_json::{Value, json};

fn decode(schema: &str, data: Value) -> Result<Value, serde_json::Error> {
    macro_rules! decode {
        ($($ty:ty),* $(,)?) => { match schema {
            $(stringify!($ty) => serde_json::from_value::<$ty>(data).and_then(serde_json::to_value),)*
            _ => panic!("unhandled fixture schema: {schema}"),
        } };
    }
    decode!(
        RootId,
        EntryName,
        RootPath,
        ByteCount,
        ManifestId,
        RetentionPolicy,
        DirectoryDraftBase,
        DraftEntryReference,
        ErrorResponse,
        Placement,
        BackendInspection,
        EntryPage,
        AllocationInspection,
        UploadInspection,
        PageSize,
        Milliseconds,
        VersionCount,
        ChunkDigest,
        CursorToken,
        RenameEntryRequest
    )
}

#[test]
fn shared_wire_fixtures_round_trip_or_reject() {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/wire.json")).unwrap();
    for case in fixtures["valid"].as_array().unwrap() {
        let output = decode(case["schema"].as_str().unwrap(), case["data"].clone())
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        assert_eq!(output, case["data"], "{}", case["name"]);
    }
    for case in fixtures["invalid"].as_array().unwrap() {
        assert!(
            decode(case["schema"].as_str().unwrap(), case["data"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
}

fn mirror() -> Placement {
    let fixtures: Value = serde_json::from_str(include_str!("fixtures/wire.json")).unwrap();
    serde_json::from_value(
        fixtures["valid"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["schema"] == "Placement")
            .unwrap()["data"]
            .clone(),
    )
    .unwrap()
}

#[test]
fn mirror_composition_and_read_order_are_provider_independent() {
    let mut value = mirror();
    value.validate().unwrap();
    if let Placement::Mirror {
        backend_ids,
        read_policy,
    } = &mut value
    {
        // Read order is deliberately different from placement member order.
        assert_ne!(read_policy.candidates[0].backend_id, backend_ids[0]);
        read_policy.candidates[1].backend_id = read_policy.candidates[0].backend_id.clone();
    }
    assert!(value.validate().is_err());
    let mut value = mirror();
    if let Placement::Mirror { backend_ids, .. } = &mut value {
        backend_ids[1] = backend_ids[0].clone();
    }
    assert!(value.validate().is_err());
    let mut value = mirror();
    if let Placement::Mirror { read_policy, .. } = &mut value {
        read_policy.candidates.clear();
    }
    assert!(value.validate().is_err());
}

#[test]
fn listing_cursor_is_bound_to_its_directory_revision() {
    let mut page = EntryPage {
        directory_id: "dir_a".parse().unwrap(),
        directory_revision: "rev_a".parse().unwrap(),
        items: vec![],
        next_cursor: Some(DirectoryCursor {
            token: "cursor_a".parse().unwrap(),
            directory_id: "dir_a".parse().unwrap(),
            directory_revision: "rev_b".parse().unwrap(),
        }),
    };
    assert!(page.validate().is_err());
    page.next_cursor.as_mut().unwrap().directory_revision = page.directory_revision.clone();
    page.validate().unwrap();
    let error = ErrorResponse::new(
        "request_a".parse().unwrap(),
        ErrorCode::CursorRefreshRequired,
    );
    assert_eq!(
        error.retry,
        RetryGuidance::Refresh {
            scope: RevisionScope::Directory
        }
    );
}

#[test]
fn busy_errors_are_retryable_and_fixed_text_cannot_leak_backend_details() {
    let error = ErrorResponse::new("request_a".parse().unwrap(), ErrorCode::RootBusy);
    assert_eq!(error.code.http_status(), 409);
    assert_eq!(
        error.retry,
        RetryGuidance::After {
            delay_ms: 1000.try_into().unwrap()
        }
    );
    assert_eq!(
        error.message,
        "Another operation owns this root; retry later."
    );
    assert!(error.details.is_none());
    for code in [
        ErrorCode::QuotaExhausted,
        ErrorCode::StagingExhausted,
        ErrorCode::DestinationCapacity,
    ] {
        assert_eq!(code.http_status(), 507);
    }
}

#[test]
fn destructive_policy_apply_requires_explicit_acknowledgement() {
    let request: ApplyPolicyRequest = serde_json::from_value(json!({"preview_id":"preview_a", "expected_revision":"policy_a", "expected_scope_revision":"scope_a", "acknowledge_retention_effect":false})).unwrap();
    assert!(request.validate().is_err());
    assert_eq!(
        serde_json::to_value(RetentionPolicy::default()).unwrap(),
        json!({"mode":"strict","keep_versions":1})
    );
}

#[cfg(feature = "codegen")]
#[test]
fn generation_is_deterministic_and_does_not_advertise_planned_routes() {
    let a = sct_protocol::codegen::artifacts().unwrap();
    assert_eq!(a, sct_protocol::codegen::artifacts().unwrap());
    let openapi: Value = serde_json::from_str(&a["contracts/openapi.json"]).unwrap();
    assert!(!openapi["paths"].as_object().unwrap().is_empty());
    assert!(openapi["components"]["schemas"].as_object().unwrap().len() > 60);
    let mut ids = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    for operation in openapi["x-sct-planned-operations"].as_array().unwrap() {
        let implemented =
            FOUNDATION_OPERATIONS.contains(&operation["operation_id"].as_str().unwrap());
        assert_eq!(operation["implemented"], implemented);
        let path = operation["path"].as_str().unwrap();
        let method = operation["method"].as_str().unwrap().to_lowercase();
        assert_eq!(openapi["paths"][path].get(&method).is_some(), implemented);
        assert!(operation["path"].as_str().unwrap().starts_with("/api/v1/"));
        assert!(ids.insert(operation["operation_id"].as_str().unwrap()));
        assert!(paths.insert((
            operation["method"].as_str().unwrap(),
            operation["path"].as_str().unwrap()
        )));
        for field in ["request_schema", "response_schema"] {
            if let Some(name) = operation[field].as_str() {
                assert!(
                    openapi["components"]["schemas"].get(name).is_some(),
                    "missing {name}"
                );
            }
        }
    }
}

#[test]
fn stale_request_cursor_requires_refresh_and_upload_receipts_require_publication() {
    let request = ListEntriesRequest {
        limit: 20.try_into().unwrap(),
        include_hidden: false,
        cursor: Some(DirectoryCursor {
            token: "cursor_a".parse().unwrap(),
            directory_id: "dir_a".parse().unwrap(),
            directory_revision: "old".parse().unwrap(),
        }),
    };
    assert_eq!(
        request.validate_cursor(&"dir_a".parse().unwrap(), &"new".parse().unwrap()),
        Err(ErrorCode::CursorRefreshRequired)
    );
    let mut upload: UploadInspection = serde_json::from_value(json!({
        "id":"upload_a", "root_id":"root_a", "operation_id":"op_a", "state":"open", "expires_at_ms":1800000000000u64,
        "copies":[{"backend_id":"backend_a","acknowledged_bytes":1024,"state":"acknowledged"}, {"backend_id":"backend_b","acknowledged_bytes":0,"state":"unavailable"}], "result":null
    })).unwrap();
    upload.validate().unwrap();
    upload.state = SessionState::Ready;
    assert!(upload.validate().is_err());
    upload.copies[1].state = CopyAcknowledgement::Acknowledged;
    upload.validate().unwrap();
    upload.state = SessionState::Committed;
    assert!(upload.validate().is_err());
    upload.result = Some(CommitReceipt {
        version_id: "version_a".parse().unwrap(),
        manifest_id: "a".repeat(64).parse().unwrap(),
        commit_sequence: 1.try_into().unwrap(),
    });
    upload.validate().unwrap();
}

#[cfg(feature = "codegen")]
#[test]
fn drift_checker_catches_modified_and_missing_artifacts_in_fresh_processes() {
    use std::{fs, process::Command};
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let directory_name = format!(
        "sct-contracts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = Scratch(std::env::temp_dir().join(directory_name));
    fs::create_dir(&root.0).unwrap();
    let run = |mode| {
        Command::new(env!("CARGO_BIN_EXE_sct-contracts"))
            .arg(mode)
            .arg(&root.0)
            .output()
            .unwrap()
    };
    assert!(run("write").status.success());
    let first = fs::read(root.0.join("contracts/openapi.json")).unwrap();
    assert!(run("write").status.success());
    assert_eq!(
        first,
        fs::read(root.0.join("contracts/openapi.json")).unwrap()
    );
    assert!(run("check").status.success());
    fs::write(root.0.join("web/src/generated/api.ts"), "drift").unwrap();
    let failure = run("check");
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("web/src/generated/api.ts"));
    assert_eq!(
        fs::read_to_string(root.0.join("web/src/generated/api.ts")).unwrap(),
        "drift",
        "check must not rewrite files"
    );
    assert!(run("write").status.success());
    fs::remove_file(root.0.join("contracts/schema.json")).unwrap();
    assert!(!run("check").status.success());
}
