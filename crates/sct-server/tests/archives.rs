#[path = "../../sct-core/tests/support/mod.rs"]
mod support;
use sct_core::backend::Registry;
use sct_protocol::*;
use serde_json::json;
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn browser_archives_require_csrf_keep_account_isolation_and_forbid_source_mutation() {
    let (catalog, _) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let admin_token = writer.recover_administrator().await.unwrap();
    let admin = writer
        .authenticate(&admin_token.secret, false, None)
        .await
        .unwrap();
    writer
        .create_principal(&admin, &"user".parse().unwrap())
        .await
        .unwrap();
    let token = writer
        .issue_token(&admin, &"user".parse().unwrap())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry: Registry::default(),
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: None,
    };
    let server = tokio::spawn(async move {
        simple_server::axum::serve(listener, sct_server::application(app, "nonexistent".into()))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let client = sct_client::Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(token.secret.clone())
        .unwrap();
    assert!(!client.archive_service_status().await.unwrap().configured);
    let limits = client.archive_limits().await.unwrap();
    assert_eq!(limits.principal_id.as_str(), "user");
    let request = UpdateArchiveAccountLimitsRequest {
        expected_revision: limits.revision.clone(),
        quota_bytes: 1_000_000.try_into().unwrap(),
        pin_quota_bytes: limits.pin_quota_bytes,
        max_active: limits.max_active,
        max_capture_records: limits.max_capture_records,
        pin_deadline_seconds: limits.pin_deadline_seconds,
        cache_ttl_seconds: 3600,
    };
    assert!(
        matches!(client.update_archive_account_limits(&limits.principal_id, &request).await, Err(sct_client::Error::Server(e)) if e.code==ErrorCode::Forbidden)
    );
    let admin_client = sct_client::Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(admin_token.secret.clone())
        .unwrap();
    let changed = admin_client
        .update_archive_account_limits(&limits.principal_id, &request)
        .await
        .unwrap();
    assert_eq!(changed.quota_bytes.get(), 1_000_000);
    assert_eq!(changed.cache_ttl_seconds, 3600);
    assert_ne!(changed.revision, limits.revision);
    assert!(
        matches!(admin_client.update_archive_account_limits(&limits.principal_id, &request).await, Err(sct_client::Error::Server(e)) if e.code==ErrorCode::RevisionConflict)
    );

    let session = http
        .post(format!("{origin}/auth/session"))
        .bearer_auth(&token.secret)
        .header("origin", &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 200);
    let cookies: Vec<_> = session
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .collect();
    let csrf = cookies
        .iter()
        .find_map(|c| c.strip_prefix("sct_csrf="))
        .unwrap();
    let cookie = cookies.join("; ");
    let body =
        json!({"request_id":"collection","label":"Optical","media_capacity_bytes":25000000000u64});
    assert_eq!(
        http.post(format!("{origin}/api/v1/archives"))
            .header("cookie", &cookie)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = http
        .post(format!("{origin}/api/v1/archives"))
        .header("cookie", &cookie)
        .header("origin", &origin)
        .header("x-csrf-token", csrf)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let collection = response.json::<ArchiveCollection>().await.unwrap();
    assert_eq!(collection.owner.as_str(), "user");
    let page = http
        .get(format!("{origin}/api/v1/archives?limit=1"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json::<ArchiveCollectionPage>()
        .await
        .unwrap();
    assert_eq!(page.collections.len(), 1);
    writer
        .create_principal(&admin, &"stranger".parse().unwrap())
        .await
        .unwrap();
    let stranger = writer
        .issue_token(&admin, &"stranger".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/archives/{}",
            collection.id.as_str()
        ))
        .bearer_auth(&stranger.secret)
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    assert_eq!(
        http.get(format!(
            "{origin}/api/v1/archives/{}",
            collection.id.as_str()
        ))
        .bearer_auth(&admin_token.secret)
        .send()
        .await
        .unwrap()
        .status(),
        200
    );
    // The existing session boundary remains in force even for administrators.
    for path in [
        "/api/v1/roots/root/uploads",
        "/api/v1/roots/root/tree-drafts",
    ] {
        assert_eq!(
            http.post(format!("{origin}{path}"))
                .header("cookie", &cookie)
                .header("origin", &origin)
                .header("x-csrf-token", csrf)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        http.get(format!("{origin}/api/v1/archives?limit=1001"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    server.abort();
    writer.release().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL and xorriso"]
async fn iso_http_ranges_resume_headers_and_expiry_are_consistent() {
    use sct_core::{archive_export::ArchiveExportConfig, transfer_io::TransferEngine};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    let (catalog, db) = support::database().await;
    let writer = catalog.acquire_writer().await.unwrap();
    let token = writer.recover_administrator().await.unwrap();
    let actor = writer
        .authenticate(&token.secret, false, None)
        .await
        .unwrap();
    let owner = actor.principal_id();
    let directory =
        std::env::temp_dir().join(format!("sct-download-http-{}", uuid::Uuid::new_v4()));
    let bundle_dir = directory.join("bundle");
    std::fs::create_dir_all(bundle_dir.join("recovery")).unwrap();
    // Transport fixture only. Real executable/ISO validation lives in the core
    // archive_export tests; this fixture isolates HTTP semantics from generation.
    let bytes = b"fixture";
    let hash = format!("{:x}", Sha256::digest(bytes));
    let mut files = Vec::new();
    for path in [
        "START-HERE.txt",
        "recovery/sct-recover",
        "recovery/source.tar.gz",
    ] {
        std::fs::write(bundle_dir.join(path), bytes).unwrap();
        files.push(ArchiveBundleFile {
            path: path.into(),
            length: (bytes.len() as u64).try_into().unwrap(),
            sha256: hash.parse().unwrap(),
        });
    }
    let bundle = ArchiveBundle {
        format: 1,
        version: "http-test".into(),
        platform: "linux-x86_64-glibc236".into(),
        files,
    };
    std::fs::write(
        bundle_dir.join("recovery/bundle.json"),
        canonical(&bundle).unwrap(),
    )
    .unwrap();
    let export_dir = directory.join("exports");
    let engine = TransferEngine::new(writer.clone(), BTreeMap::new(), 2, 3600)
        .unwrap()
        .with_archive_exports(ArchiveExportConfig {
            directory: export_dir.clone(),
            bundle_directory: bundle_dir,
            quota_bytes: 1_000_000.try_into().unwrap(),
        })
        .await
        .unwrap();
    let body = b"immutable ISO transport fixture";
    let digest = format!("{:x}", Sha256::digest(body));
    let artifact = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(export_dir.join(format!("cache/{artifact}.iso")), body).unwrap();
    let mut tx = db.begin().await.unwrap();
    sqlx::query("INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision) VALUES('source',$1,'source','/','r','enabled','{}',4000,'{}','source','r')").bind(owner).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO archive_accounts(principal_id,cache_bytes) VALUES($1,$2)")
        .bind(owner)
        .bind(body.len() as i64)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE archive_storage SET cache_bytes=$1")
        .bind(body.len() as i64)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO archive_collections(id,owner_id,label,revision,media_capacity,next_disc,next_set) VALUES('collection',$1,'collection','r',1000000,2,2)").bind(owner).execute(&mut *tx).await.unwrap();
    let plan = json!({"source_files":0,"source_bytes":0,"new_payload_bytes":0,"reused_payload_bytes":0,"new_disc_count":1,"required_earlier_disc_count":0,"index_bytes_per_disc":1,"bundle_bytes_per_disc":1,"filesystem_reserve_per_disc":1,"total_iso_budget_bytes":1000000,"bundle_sha256":hash});
    sqlx::query("INSERT INTO archive_sets(id,collection_id,sequence,revision,state,principal_id,credential_id,selections,media_capacity,plan,first_disc,last_disc,frozen_at,published_at) VALUES('set','collection',1,'r','ready',$1,$2,$3,1000000,$4,1,1,clock_timestamp(),clock_timestamp())").bind(owner).bind(&token.id).bind(json!([{"root_id":"source","version_id":"version","path":"/"}])).bind(plan).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL sct.archive_import='on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO archive_discs(id,collection_id,set_id,number,index_sha256,bundle_sha256,inventory_sha256,payload_bytes,object_count,planned_iso_bytes) VALUES('disc','collection','set',1,$1,$1,$1,0,0,1000000)").bind(&hash).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO archive_artifacts(id,disc_id,state,sha256,length,relative_path,expires_at) VALUES($1,'disc','available',$2,$3,$4,clock_timestamp()+interval '1 hour')").bind(&artifact).bind(&digest).bind(body.len() as i64).bind(format!("cache/{artifact}.iso")).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = sct_server::api::App {
        writer: writer.clone(),
        registry: Registry::default(),
        oidc: None,
        public_url: origin.clone(),
        secure_cookies: false,
        transfer: Some(engine.clone()),
    };
    let server = tokio::spawn(async move {
        simple_server::axum::serve(listener, sct_server::application(app, "nonexistent".into()))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let url = format!("{origin}/api/v1/archives/collection/discs/disc/iso");
    let etag = format!("\"{digest}\"");
    let head = http
        .head(&url)
        .bearer_auth(&token.secret)
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()["etag"], etag);
    assert_eq!(head.headers()["content-length"], body.len().to_string());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT download_count FROM archive_artifacts")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(http.get(&url).send().await.unwrap().status(), 401);
    for (range, status, expected) in [
        ("bytes=2-5", 206, &body[2..6]),
        ("bytes=-3", 206, &body[body.len() - 3..]),
        ("bytes=999-", 416, &b""[..]),
    ] {
        let response = http
            .get(&url)
            .bearer_auth(&token.secret)
            .header("range", range)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == 206 {
            assert_eq!(response.headers()["etag"], etag);
            assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
        } else {
            assert_eq!(
                response.headers()["content-range"],
                format!("bytes */{}", body.len())
            );
        }
    }
    assert_eq!(
        http.get(&url)
            .bearer_auth(&token.secret)
            .header("if-match", "\"wrong\"")
            .send()
            .await
            .unwrap()
            .status(),
        412
    );
    let response = http
        .get(&url)
        .bearer_auth(&token.secret)
        .header("range", "bytes=2-")
        .header("if-range", "\"wrong\"")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), body);
    let client = sct_client::Client::new(origin.parse().unwrap())
        .unwrap()
        .with_token(token.secret.clone())
        .unwrap();
    let info = engine
        .archive_image_info(
            &actor,
            &"collection".parse().unwrap(),
            &"disc".parse().unwrap(),
        )
        .await
        .unwrap();
    let mut partial = std::io::Cursor::new(body[..5].to_vec());
    client
        .resume_archive_iso_to(
            &"collection".parse().unwrap(),
            &"disc".parse().unwrap(),
            &info.artifact,
            &mut partial,
        )
        .await
        .unwrap();
    assert_eq!(partial.into_inner(), body);
    sqlx::query("UPDATE archive_artifacts SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        http.get(&url)
            .bearer_auth(&token.secret)
            .send()
            .await
            .unwrap()
            .status(),
        410
    );
    engine.cleanup_archive_cache().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archive_discs")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    let copy_request = RegisterArchiveCopyRequest {
        request_id: "copy".parse().unwrap(),
        label: "Offsite copy".parse().unwrap(),
        location: "Shelf 4 / top".into(),
        reported_burned_at_ms: None,
    };
    let collection = "collection".parse().unwrap();
    let disc = "disc".parse().unwrap();
    let copy = client
        .register_archive_copy(&collection, &disc, &copy_request)
        .await
        .unwrap();
    assert_eq!(
        copy,
        client
            .register_archive_copy(&collection, &disc, &copy_request)
            .await
            .unwrap()
    );
    let report = SubmitArchiveVerificationRequest {
        request_id: "report".parse().unwrap(),
        expected_revision: copy.revision.clone(),
        collection_id: collection.clone(),
        disc_id: disc.clone(),
        index_sha256: hash.parse().unwrap(),
        bundle_sha256: hash.parse().unwrap(),
        payload_inventory_sha256: hash.parse().unwrap(),
        expected_objects: 0.try_into().unwrap(),
        verified_objects: 0.try_into().unwrap(),
        verified_bytes: 0.try_into().unwrap(),
        metadata_verified: true,
        verifier_version: "http-fixture".parse().unwrap(),
    };
    let report = client
        .submit_archive_verification(&collection, &copy.id, &report)
        .await
        .unwrap();
    assert!(report.complete_read_back);
    let page = ListRequest {
        limit: 10.try_into().unwrap(),
        cursor: None,
    };
    let copy = client
        .list_archive_copies(&collection, &disc, &page)
        .await
        .unwrap()
        .copies
        .pop()
        .unwrap();
    let updated = client
        .update_archive_copy(
            &collection,
            &copy.id,
            &UpdateArchiveCopyRequest {
                expected_revision: copy.revision.clone(),
                label: copy.label.clone(),
                location: copy.location.clone(),
                reported_burned_at_ms: None,
                health: Some(ArchiveCopyHealth::Damaged),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.health, ArchiveCopyHealth::Damaged);
    assert_eq!(
        client
            .list_archive_reports(&collection, &copy.id, &page)
            .await
            .unwrap()
            .reports
            .len(),
        1
    );
    assert_eq!(
        client
            .list_archive_copy_events(&collection, &copy.id, &page)
            .await
            .unwrap()
            .events
            .len(),
        3
    );
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(directory).unwrap();
}
