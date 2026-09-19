//! Deterministic exports from Rust-owned definitions. Enabled only for tooling.
use crate::*;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use ts_rs::{Config, TS};

fn register_types(
    settings: SchemaSettings,
    config: &Config,
    declarations: &mut BTreeMap<String, String>,
) -> schemars::SchemaGenerator {
    let mut schemas = settings.into_generator();
    fn register<T: JsonSchema + TS>(
        schemas: &mut schemars::SchemaGenerator,
        config: &Config,
        declarations: &mut BTreeMap<String, String>,
    ) {
        let _ = schemas.subschema_for::<T>();
        declarations.insert(T::ident(config), format!("export {}", T::decl(config)));
    }
    macro_rules! types { ($target:ident, $($ty:ty),* $(,)?) => { $(register::<$ty>(&mut $target, config, declarations);)* }; }
    types!(
        schemas,
        ChunkReference,
        PageReference,
        ManifestPage,
        FileManifest,
        FileTarget,
        CreateUploadRequest,
        AdmissionState,
        TransferStatus,
        MissingChunkPage,
        FileVersion,
        VersionPage,
        RequestId,
        PrincipalId,
        CredentialId,
        GrantId,
        RootId,
        EntryId,
        VersionId,
        BackendId,
        PoolId,
        AllocationId,
        UploadId,
        OperationId,
        PolicyId,
        PreviewId,
        CheckpointId,
        Revision,
        IdempotencyKey,
        CursorToken,
        ManifestId,
        ChunkDigest,
        EntryName,
        RootPath,
        ByteCount,
        CommitSequence,
        TimestampMs,
        PageSize,
        VersionCount,
        Milliseconds,
        BytesPerSecond,
        Capability,
        ErrorCode,
        RevisionScope,
        RetryGuidance,
        ErrorDetails,
        ErrorResponse,
        RequestLimits,
        Capabilities,
        ListRequest,
        DirectoryCursor,
        ListEntriesRequest,
        RetentionMode,
        RetentionPolicy,
        VersioningMode,
        PolicyScope,
        EffectivePolicy,
        PreviewPolicyRequest,
        PolicyPreview,
        ApplyPolicyRequest,
        AvailabilityHint,
        ObservedHealth,
        InstanceHints,
        AdapterDocumentation,
        BackendInspection,
        ReadCandidate,
        ReadPolicy,
        Placement,
        RootState,
        RootInspection,
        RootPage,
        CreateRootRequest,
        UpdateRootRequest,
        GrantInspection,
        UpdateGrantRequest,
        EntryKind,
        EntryInspection,
        EntryPage,
        ResolvePathRequest,
        CreateDirectoryRequest,
        RenameEntryRequest,
        RootAllocationRequest,
        AllocationOwner,
        AllocationInspection,
        PoolInspection,
        RootUsage,
        DestinationUsage,
        CapacityRecovery,
        OperationKind,
        OperationState,
        OperationInspection,
        SessionState,
        CopyAcknowledgement,
        CopyProgress,
        CommitReceipt,
        UploadInspection,
        DraftMode,
        DraftEntryIdentity,
        DirectoryDraftBase,
        DraftEntryReference,
        CreateGrantRequest,
        GrantPage,
        SetPolicyRequest,
        ListEntriesQuery,
        ListGrantsQuery,
        ExpectedRevision,
        SessionInspection,
        IssuedToken,
        AllocationPage,
        PolicyConstraintsRequest
    );
    schemas
}

pub fn artifacts() -> Result<BTreeMap<&'static str, String>, serde_json::Error> {
    let config = Config::default();
    let mut declarations = BTreeMap::new();
    let schemas = register_types(SchemaSettings::draft2020_12(), &config, &mut declarations);
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://sct.invalid/contracts/v1",
        "$defs": schemas.definitions(),
    });
    let mut openapi_settings = SchemaSettings::draft2020_12();
    openapi_settings.definitions_path = "/components/schemas".into();
    let schemas = register_types(openapi_settings, &config, &mut declarations);
    let operations = operations();
    let openapi = json!({
        "openapi": "3.1.0",
        "info": {"title": "ScT v1 independent-file storage", "version": "0.1.0", "description": "Foundation and independent-file transfer API. Only implemented routes appear in paths. Health and OIDC/session endpoints are outside the versioned application API."},
        "paths": implemented_paths(&operations, schemas.definitions()),
        "components": {"schemas": schemas.definitions(), "securitySchemes": {"bearer": {"type":"http","scheme":"bearer"}, "session": {"type":"apiKey","in":"cookie","name":"__Host-sct_session"}}},
        "security": [{"bearer": []}, {"session": []}],
        "x-sct-planned-operations": operations,
        "x-sct-request-id-header": "X-Request-Id",
        "x-sct-reserved-storage-format-version": 1,
        "x-sct-storage-format-status": "immutable file format v1; FastCDC profile pinned",
    });
    let mut typescript = String::from(
        "// Generated by sct-contracts. DO NOT EDIT.\n// IDs are nominal strings; validate untrusted JSON before using these types.\n\n",
    );
    for declaration in declarations.values() {
        typescript.push_str(declaration);
        typescript.push_str("\n\n");
    }
    let mut markdown = String::from(
        "# v1 operation map\n\nGenerated from `sct-protocol`. Foundation routes require the configured database-backed service.\nUnimplemented routes return a structured 404 and are absent from OpenAPI paths and capabilities.\n\n| Operation | Method | Path | Request | Response | Milestone | Implemented |\n| --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for op in &operations {
        markdown.push_str(&format!(
            "| {} | {} | `{}` | {} | {} | {} | {} |\n",
            op["operation_id"].as_str().unwrap(),
            op["method"].as_str().unwrap(),
            op["path"].as_str().unwrap(),
            op["request_schema"].as_str().unwrap_or("reserved"),
            op["response_schema"].as_str().unwrap_or("reserved"),
            op["milestone"].as_str().unwrap(),
            op["implemented"].as_bool().unwrap()
        ));
    }
    Ok(BTreeMap::from([
        (
            "contracts/openapi.json",
            serde_json::to_string_pretty(&openapi)? + "\n",
        ),
        (
            "contracts/schema.json",
            serde_json::to_string_pretty(&schema)? + "\n",
        ),
        ("contracts/operations.md", markdown),
        (
            "web/src/generated/api.ts",
            typescript.trim_end().to_owned() + "\n",
        ),
    ]))
}

/// Reserved operations have no wire body yet. Extend this map when contracts land.
pub fn operations() -> Vec<Value> {
    let mut result = Vec::new();
    macro_rules! op {
        ($id:literal, $method:literal, $path:literal, $request:expr, $response:expr, $milestone:literal) => {
            result.push(json!({"operation_id": $id, "method": $method, "path": concat!("/api/v1", $path), "request_schema": $request, "response_schema": $response, "milestone": $milestone, "implemented": (FOUNDATION_OPERATIONS.contains(&$id) || TRANSFER_OPERATIONS.contains(&$id))}));
        };
    }
    op!(
        "capabilities",
        "GET",
        "/capabilities",
        None::<&str>,
        "Capabilities",
        "M1"
    );
    op!(
        "list_roots",
        "GET",
        "/roots",
        "ListRequest",
        "RootPage",
        "M1"
    );
    op!(
        "create_root",
        "POST",
        "/roots",
        "CreateRootRequest",
        "RootInspection",
        "M1"
    );
    op!(
        "inspect_root",
        "GET",
        "/roots/{root}",
        None::<&str>,
        "RootInspection",
        "M1"
    );
    op!(
        "update_root",
        "PATCH",
        "/roots/{root}",
        "UpdateRootRequest",
        "RootInspection",
        "M1"
    );
    op!(
        "list_entries",
        "GET",
        "/roots/{root}/entries",
        "ListEntriesQuery",
        "EntryPage",
        "M1"
    );
    op!(
        "resolve_path",
        "GET",
        "/roots/{root}/resolve",
        "ResolvePathRequest",
        "EntryInspection",
        "M1"
    );
    op!(
        "create_directory",
        "POST",
        "/roots/{root}/entries",
        "CreateDirectoryRequest",
        "EntryInspection",
        "M1"
    );
    op!(
        "inspect_entry",
        "GET",
        "/entries/{entry}",
        None::<&str>,
        "EntryInspection",
        "M1"
    );
    op!(
        "rename_entry",
        "POST",
        "/entries/{entry}/rename",
        "RenameEntryRequest",
        "EntryInspection",
        "M1"
    );
    op!(
        "effective_policy",
        "GET",
        "/policies",
        "PolicyScope",
        "EffectivePolicy",
        "M1"
    );
    op!(
        "preview_policy",
        "POST",
        "/policies/preview",
        "PreviewPolicyRequest",
        "PolicyPreview",
        "M3"
    );
    op!(
        "apply_policy",
        "POST",
        "/policies/apply",
        "ApplyPolicyRequest",
        "OperationInspection",
        "M3"
    );
    op!(
        "root_operation",
        "GET",
        "/roots/{root}/operation",
        None::<&str>,
        "OperationInspection",
        "M1"
    );
    op!(
        "inspect_usage",
        "GET",
        "/roots/{root}/usage",
        None::<&str>,
        "RootUsage",
        "M2"
    );
    op!(
        "inspect_pool",
        "GET",
        "/admin/pools/{pool}",
        None::<&str>,
        "PoolInspection",
        "M1"
    );
    op!(
        "inspect_allocation",
        "GET",
        "/admin/allocations/{allocation}",
        None::<&str>,
        "AllocationInspection",
        "M1"
    );
    op!(
        "inspect_backend",
        "GET",
        "/admin/backends/{backend}",
        None::<&str>,
        "BackendInspection",
        "M1"
    );
    op!(
        "adapter_documentation",
        "GET",
        "/admin/adapters/{kind}",
        None::<&str>,
        "AdapterDocumentation",
        "M1"
    );
    op!(
        "inspect_grant",
        "GET",
        "/admin/grants/{grant}",
        None::<&str>,
        "GrantInspection",
        "M1"
    );
    op!(
        "update_grant",
        "PATCH",
        "/admin/grants/{grant}",
        "UpdateGrantRequest",
        "GrantInspection",
        "M1"
    );
    op!(
        "hide_entry",
        "DELETE",
        "/entries/{entry}",
        None::<&str>,
        None::<&str>,
        "M3"
    );
    op!(
        "purge_entry",
        "POST",
        "/entries/{entry}/purge",
        None::<&str>,
        None::<&str>,
        "M3"
    );
    op!(
        "list_versions",
        "GET",
        "/roots/{root}/entries/{entry}/versions",
        None::<&str>,
        "VersionPage",
        "M2"
    );
    op!(
        "read_manifest",
        "GET",
        "/roots/{root}/versions/{version}/manifest",
        None::<&str>,
        "FileManifest",
        "M2"
    );
    op!(
        "download_content",
        "GET",
        "/roots/{root}/versions/{version}/content",
        None::<&str>,
        None::<&str>,
        "M2"
    );
    op!(
        "begin_upload",
        "POST",
        "/roots/{root}/uploads",
        "CreateUploadRequest",
        "TransferStatus",
        "M2"
    );
    op!(
        "inspect_upload",
        "GET",
        "/roots/{root}/uploads/{upload}",
        None::<&str>,
        "TransferStatus",
        "M2"
    );
    op!(
        "abort_upload",
        "POST",
        "/roots/{root}/uploads/{upload}/abort",
        None::<&str>,
        "TransferStatus",
        "M2"
    );
    op!(
        "missing_chunks",
        "GET",
        "/roots/{root}/uploads/{upload}/chunks",
        None::<&str>,
        "MissingChunkPage",
        "M2"
    );
    op!(
        "upload_chunk",
        "PUT",
        "/roots/{root}/uploads/{upload}/chunks/{digest}",
        None::<&str>,
        None::<&str>,
        "M2"
    );
    op!(
        "upload_page",
        "PUT",
        "/roots/{root}/uploads/{upload}/pages/{digest}",
        None::<&str>,
        None::<&str>,
        "M2"
    );
    op!(
        "admit_upload",
        "POST",
        "/roots/{root}/uploads/{upload}/admit",
        None::<&str>,
        "TransferStatus",
        "M2"
    );
    op!(
        "seal_upload",
        "POST",
        "/roots/{root}/uploads/{upload}/seal",
        None::<&str>,
        "TransferStatus",
        "M2"
    );
    op!(
        "commit_upload",
        "POST",
        "/roots/{root}/uploads/{upload}/commit",
        None::<&str>,
        "CommitReceipt",
        "M2"
    );
    op!(
        "list_principals",
        "GET",
        "/admin/principals",
        "ListRequest",
        None::<&str>,
        "M4"
    );
    op!(
        "create_principal",
        "POST",
        "/admin/principals",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "create_credential",
        "POST",
        "/admin/credentials",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "revoke_credential",
        "DELETE",
        "/admin/credentials/{credential}",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "create_grant",
        "POST",
        "/admin/grants",
        "CreateGrantRequest",
        "GrantInspection",
        "M1"
    );
    op!(
        "list_grants",
        "GET",
        "/admin/grants",
        "ListGrantsQuery",
        "GrantPage",
        "M1"
    );
    op!(
        "revoke_grant",
        "DELETE",
        "/admin/grants/{grant}",
        "ExpectedRevision",
        None::<&str>,
        "M1"
    );
    op!(
        "list_backends",
        "GET",
        "/admin/backends",
        "ListRequest",
        None::<&str>,
        "M4"
    );
    op!(
        "configure_backend",
        "PUT",
        "/admin/backends/{backend}",
        None::<&str>,
        "BackendInspection",
        "M4"
    );
    op!(
        "list_pools",
        "GET",
        "/admin/pools",
        "ListRequest",
        None::<&str>,
        "M4"
    );
    op!(
        "configure_pool",
        "PUT",
        "/admin/pools/{pool}",
        None::<&str>,
        "PoolInspection",
        "M4"
    );
    op!(
        "list_allocations",
        "GET",
        "/admin/allocations",
        "ListRequest",
        "AllocationPage",
        "M1"
    );
    op!(
        "configure_allocation",
        "PUT",
        "/admin/allocations/{allocation}",
        None::<&str>,
        "AllocationInspection",
        "M4"
    );
    op!(
        "list_jobs",
        "GET",
        "/admin/jobs",
        "ListRequest",
        None::<&str>,
        "M4"
    );
    op!(
        "inspect_job",
        "GET",
        "/admin/jobs/{job}",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "start_integrity_scan",
        "POST",
        "/admin/roots/{root}/integrity",
        None::<&str>,
        "OperationInspection",
        "M4"
    );
    op!(
        "start_repair",
        "POST",
        "/admin/roots/{root}/repair",
        None::<&str>,
        "OperationInspection",
        "M4"
    );
    op!(
        "list_audit",
        "GET",
        "/admin/audit",
        "ListRequest",
        None::<&str>,
        "M4"
    );
    op!(
        "create_checkpoint",
        "POST",
        "/admin/checkpoints",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "inspect_checkpoint",
        "GET",
        "/admin/checkpoints/{checkpoint}",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "finish_checkpoint",
        "POST",
        "/admin/checkpoints/{checkpoint}/finish",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "abandon_checkpoint",
        "POST",
        "/admin/checkpoints/{checkpoint}/abandon",
        None::<&str>,
        None::<&str>,
        "M4"
    );
    op!(
        "set_policy",
        "PATCH",
        "/policies",
        "SetPolicyRequest",
        "EffectivePolicy",
        "M1"
    );
    op!(
        "policy_constraints",
        "PATCH",
        "/admin/roots/{root}/policy-constraints",
        "PolicyConstraintsRequest",
        "RootInspection",
        "M1"
    );
    result
}

fn implemented_paths(operations: &[Value], definitions: &serde_json::Map<String, Value>) -> Value {
    let mut paths = serde_json::Map::<String, Value>::new();
    for op in operations.iter().filter(|op| op["implemented"] == true) {
        let method = op["method"].as_str().unwrap();
        let path = op["path"].as_str().unwrap();
        let mut parameters = Vec::new();
        for segment in path.split('/') {
            if let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                parameters.push(json!({"name":name,"in":"path","required":true,"schema":{"type":"string","minLength":1,"maxLength":128}}));
            }
        }
        let mut entry = json!({"operationId":op["operation_id"],"responses":{}});
        if let Some(request) = op["request_schema"].as_str() {
            if method == "GET" || method == "DELETE" {
                if request == "PolicyScope" {
                    parameters.extend([
                        json!({"name":"kind","in":"query","required":true,"schema":{"type":"string","enum":["root","entry"]}}),
                        json!({"name":"root_id","in":"query","required":true,"schema":{"$ref":"#/components/schemas/RootId"}}),
                        json!({"name":"entry_id","in":"query","required":false,"schema":{"$ref":"#/components/schemas/EntryId"}}),
                    ]);
                } else if let Some(properties) = definitions[request]["properties"].as_object() {
                    let required = definitions[request]["required"].as_array();
                    for (name, schema) in properties {
                        parameters.push(json!({"name":name,"in":"query","required":required.is_some_and(|r|r.contains(&json!(name))),"schema":schema}));
                    }
                }
            } else {
                entry["requestBody"] = json!({"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{request}")}}}});
            }
        }
        if !matches!(method, "GET" | "HEAD") {
            parameters.push(json!({"name":"X-CSRF-Token","in":"header","required":false,"description":"Required together with a matching Origin for cookie-authenticated mutations; obtain from the sct_csrf cookie (with __Host- prefix on HTTPS).","schema":{"type":"string"}}));
        }
        let id = op["operation_id"].as_str().unwrap();
        if matches!(id, "list_versions" | "missing_chunks") {
            parameters.push(json!({"name":"cursor","in":"query","required":false,"schema":{"$ref":"#/components/schemas/CursorToken"}}));
        }
        if id == "download_content" {
            parameters.push(json!({"name":"Range","in":"header","required":false,"description":"One bytes range, inclusive/open-ended/suffix","schema":{"type":"string"}}));
        }
        if id == "upload_chunk" {
            entry["requestBody"] = json!({"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary","maxLength":CHUNK_MAX}}}});
        }
        if id == "upload_page" {
            entry["requestBody"] = json!({"required":true,"description":"Canonical JSON bytes; hash must match path digest","content":{"application/json":{"schema":{"$ref":"#/components/schemas/ManifestPage"}}}});
        }
        entry["parameters"] = json!(parameters);
        if let Some(response) = op["response_schema"].as_str() {
            entry["responses"]["200"] = json!({"description":"Success","content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{response}")}}}});
        } else {
            entry["responses"]["204"] = json!({"description":"Success without body"});
        }
        if matches!(id, "admit_upload" | "seal_upload") {
            entry["responses"]["202"] = entry["responses"]["200"].clone();
        }
        if id == "download_content" {
            entry["responses"].as_object_mut().unwrap().remove("204");
            for code in ["200", "206"] {
                entry["responses"][code] = json!({"description":"Verified streaming bytes; a late failure aborts the body. Check Content-Length.","headers":{"Content-Length":{"schema":{"type":"integer"}},"Content-Range":{"schema":{"type":"string"}}},"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}});
            }
            entry["responses"]["416"] = json!({"description":"Unsatisfiable byte range","content":{"application/json":{"schema":{"$ref":"#/components/schemas/ErrorResponse"}}}});
        }
        for status in [
            "400", "401", "403", "404", "409", "429", "500", "503", "507",
        ] {
            entry["responses"][status] = json!({"description":"Structured error with X-Request-Id; retryable failures include Retry-After.","content":{"application/json":{"schema":{"$ref":"#/components/schemas/ErrorResponse"}}}});
        }
        paths.entry(path.to_owned()).or_insert_with(|| json!({}))[method.to_lowercase()] = entry;
    }
    Value::Object(paths)
}
