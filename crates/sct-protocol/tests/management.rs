use sct_protocol::*;
use serde_json::json;

#[test]
fn management_right_is_additive_and_requests_reject_privilege_fields() {
    for value in [
        "list_read",
        "write",
        "manage_policy",
        "delete",
        "purge",
        "manage_grants",
        "manage_root",
    ] {
        let capability: Capability = serde_json::from_value(json!(value)).unwrap();
        assert_eq!(serde_json::to_value(capability).unwrap(), json!(value));
    }
    let request = json!({"expected_revision": "revision-1", "display_name": "backup",
        "quota_bytes": 100, "expected_upload_interval_seconds": null});
    let parsed: UpdateRootManagementRequest = serde_json::from_value(request.clone()).unwrap();
    parsed.validate().unwrap();
    for field in ["quota_ceiling_bytes", "administrator", "placement"] {
        let mut invalid = request.clone();
        invalid[field] = json!(200);
        assert!(serde_json::from_value::<UpdateRootManagementRequest>(invalid).is_err());
    }
    for interval in [0, 59, 31_536_001] {
        let mut invalid = request.clone();
        invalid["expected_upload_interval_seconds"] = json!(interval);
        assert!(
            serde_json::from_value::<UpdateRootManagementRequest>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}

#[test]
fn checkpoint_format_and_release_are_explicit() {
    let mut descriptor: CheckpointDescriptor = serde_json::from_value(json!({
        "format": 1, "checkpoint_id": "checkpoint-1", "generation_watermark": 42,
        "catalog_digest": digest(b"catalog"), "inventory_digest": digest(b"inventory"),
        "package_state": "catalog_only", "storage_format_version": 1
    }))
    .unwrap();
    descriptor.validate().unwrap();
    assert_eq!(descriptor.package_state, RecoveryPackageState::CatalogOnly);
    descriptor.format = 2;
    assert!(descriptor.validate().is_err());
    descriptor.format = 1;
    descriptor.storage_format_version = 2;
    assert!(descriptor.validate().is_err());
    for acknowledge in [false, true] {
        let request: ReleaseCheckpointRequest = serde_json::from_value(json!({
            "expected_revision": "revision-1", "acknowledge_release": acknowledge
        }))
        .unwrap();
        assert_eq!(request.validate().is_ok(), acknowledge);
    }
}
