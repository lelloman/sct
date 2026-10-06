use super::*;
use simple_server::database::postgres as db;
use std::time::Instant;

fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}
#[simple_server::test(host_runtime = true)]
#[ignore = "million-entry PostgreSQL qualification; scripts/qualify-tree-scale"]
async fn million_entry_preview_is_bounded_and_resumable() {
    let (mut w, a, db) = setup().await;
    let r = root(&w, &a, "/million", 4000).await;
    let mount = w
        .resolve_path(&a, &r.id, &"/".parse().unwrap())
        .await
        .unwrap();
    let source = mkdir(
        &w,
        &a,
        &r.id,
        &mount.id,
        "source",
        VersioningMode::IndependentFiles,
    )
    .await;
    let dest = mkdir(
        &w,
        &a,
        &r.id,
        &mount.id,
        "destination",
        VersioningMode::IndependentFiles,
    )
    .await;
    let mut previous = 0i64;
    for count in [10_000i64, 1_000_000] {
        let seed = Instant::now();
        // This is fixture loading, not a production mutation path. Avoid a
        // million identical generation increments; publish one after the bulk load.
        let mut tx = db.begin().await.unwrap();
        db::query("ALTER TABLE entries DISABLE TRIGGER USER")
            .execute(&mut *tx)
            .await
            .unwrap();
        db::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) SELECT 'scale-'||n,$1,$2,'entry-'||lpad(n::text,7,'0'),'directory','fixture' FROM generate_series($3::bigint,$4::bigint) n")
            .bind(r.id.as_str()).bind(source.id.as_str()).bind(previous+1).bind(count).execute(&mut *tx).await.unwrap();
        db::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *tx)
            .await
            .unwrap();
        db::query("ALTER TABLE entries ENABLE TRIGGER USER")
            .execute(&mut *tx)
            .await
            .unwrap();
        db::query("UPDATE roots SET mutation_serial=mutation_serial+1 WHERE id=$1")
            .bind(r.id.as_str())
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        // Fixture loading may outlast the writer lease; acquire a fresh epoch.
        let _ = w.release().await;
        w = w.catalog().acquire_writer().await.unwrap();
        let seed_ms = seed.elapsed().as_millis();
        let revision = w.inspect_root(&a, &r.id).await.unwrap().revision;
        let mut preview = w
            .preview_change(
                &a,
                &r.id,
                &PreviewChangeRequest {
                    expected_root_revision: revision,
                    change: PolicyChange::Move {
                        entry_id: source.id.clone(),
                        expected_revision: source.revision.clone(),
                        parent_id: dest.id.clone(),
                        expected_parent_revision: dest.revision.clone(),
                        name: "moved".parse().unwrap(),
                    },
                },
            )
            .await
            .unwrap();
        let start = Instant::now();
        let baseline = rss();
        let mut peak = baseline;
        let mut batches = 0;
        let mut previous_cursor = String::new();
        while preview.state == LifecycleState::Planning {
            preview = w.advance_preview(&a, &r.id, &preview.id).await.unwrap();
            batches += 1;
            if batches == 25 {
                let saved: String = db::query_scalar(
                    "SELECT after_child FROM preview_entry_work WHERE preview_id=$1",
                )
                .bind(preview.id.as_str())
                .fetch_one(&db)
                .await
                .unwrap();
                assert!(!saved.is_empty());
                w.release().await.unwrap();
                w = w.catalog().acquire_writer().await.unwrap();
                let resumed: String = db::query_scalar(
                    "SELECT after_child FROM preview_entry_work WHERE preview_id=$1",
                )
                .bind(preview.id.as_str())
                .fetch_one(&db)
                .await
                .unwrap();
                assert_eq!(resumed, saved);
                previous_cursor = resumed;
            }
            if batches % 100 == 0 {
                w.renew().await.unwrap();
                peak = peak.max(rss());
                let frontier: i64 =
                    db::query_scalar("SELECT count(*) FROM preview_entry_work WHERE preview_id=$1")
                        .bind(preview.id.as_str())
                        .fetch_one(&db)
                        .await
                        .unwrap();
                assert!(frontier <= 1);
            }
            assert!(batches <= count / 128 + 3);
        }
        assert!(!previous_cursor.is_empty());
        assert_eq!(preview.versions_to_retire.get(), 0);
        println!(
            "SCT_TREE_SCALE entries={count} batches={batches} seed_ms={seed_ms} elapsed_ms={} baseline_rss_kib={baseline} peak_rss_kib={peak} restarted_after=25",
            start.elapsed().as_millis()
        );
        // A successful final apply proves the entire million-entry walk finished.
        let engine = sct_core::transfer_io::TransferEngine::new(
            w.clone(),
            std::collections::BTreeMap::new(),
            1,
            60,
        )
        .unwrap();
        let job = engine
            .apply_change(
                &a,
                &r.id,
                &ApplyChangeRequest {
                    idempotency_key: format!("scale-{count}").parse().unwrap(),
                    preview_id: preview.id,
                    expected_root_revision: preview.root_revision,
                    acknowledge_retention_effect: true,
                },
            )
            .await
            .unwrap();
        for _ in 0..10 {
            engine.tick().await.unwrap();
        }
        assert_eq!(
            engine
                .lifecycle_job(&a, &r.id, &job.id)
                .await
                .unwrap()
                .state,
            LifecycleState::Completed
        );
        // Restore only the fixture's namespace pointer for the next larger measurement.
        db::query("UPDATE entries SET parent_id=$2,name='source',revision=$3 WHERE id=$1")
            .bind(source.id.as_str())
            .bind(mount.id.as_str())
            .bind(source.revision.as_str())
            .execute(&db)
            .await
            .unwrap();
        db::query("UPDATE entries SET revision=$2 WHERE id=$1")
            .bind(dest.id.as_str())
            .bind(dest.revision.as_str())
            .execute(&db)
            .await
            .unwrap();
        previous = count;
    }
}

// Canonical metadata fixture: repeated chunk references, not fabricated payload I/O.
// All graph edges and bytes agree. GC must visit every edge, preserve pinned copies,
// survive writer replacement, and reclaim only the unreferenced generation.
#[simple_server::test(host_runtime = true)]
#[ignore = "ten-million-reference qualification; scripts/qualify"]
async fn qualification_ten_million_references_are_reclaimed_with_bounded_work() {
    use sct_core::payload::{MemoryStore, ObjectStore};
    use sct_core::transfer_io::TransferEngine;
    use std::collections::BTreeMap;
    let count: u64 = std::env::var("SCT_QUALIFY_REFERENCES")
        .unwrap_or_else(|_| "10000000".into())
        .parse()
        .unwrap();
    assert!((10_000..=10_000_000).contains(&count));
    let (mut w, a, db) = setup().await;
    let root = root(&w, &a, "/references", 4000).await;
    let seed = Instant::now();
    // Bulk metadata loading publishes one generation change. FK checks remain on.
    let mut tx = db.begin().await.unwrap();
    db::query("ALTER TABLE content_edges DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut leaves = Vec::new();
    let mut left = count;
    let mut first_chunk = None;
    let mut index = 0u64;
    while left > 0 {
        let n = left.min(MANIFEST_FANOUT as u64);
        let payload = index.to_le_bytes();
        let chunk = digest(&payload);
        first_chunk.get_or_insert((chunk.clone(), payload.to_vec()));
        db::query(
            "INSERT INTO content_objects(root_id,digest,kind,length) VALUES($1,$2,'chunk',8)",
        )
        .bind(root.id.as_str())
        .bind(&chunk)
        .execute(&mut *tx)
        .await
        .unwrap();
        let page = ManifestPage::Chunks {
            format: 1,
            chunks: (0..n)
                .map(|_| ChunkReference {
                    digest: chunk.parse().unwrap(),
                    length: 8.try_into().unwrap(),
                })
                .collect(),
        };
        let body = canonical(&page).unwrap();
        let parent = digest(&body);
        db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,'page',$3,0,$4)")
            .bind(root.id.as_str()).bind(&parent).bind(body.len() as i64).bind(body).execute(&mut *tx).await.unwrap();
        db::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) SELECT $1,$2,n,$3,8,1 FROM generate_series(0,$4::integer-1) n")
            .bind(root.id.as_str()).bind(&parent).bind(&chunk).bind(n as i32).execute(&mut *tx).await.unwrap();
        leaves.push(PageReference {
            digest: parent.parse().unwrap(),
            length: (n * 8).try_into().unwrap(),
            chunks: n.try_into().unwrap(),
        });
        left -= n;
        index += 1;
        if index.is_multiple_of(1000) {
            println!("QUALIFY references_seeded={}", count - left);
        }
    }
    let mut height = 0;
    while leaves.len() > 1 {
        height += 1;
        let mut next = Vec::new();
        for children in leaves.chunks(MANIFEST_FANOUT) {
            let page = ManifestPage::Branch {
                format: 1,
                height,
                pages: children.to_vec(),
            };
            let body = canonical(&page).unwrap();
            let parent = digest(&body);
            db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,'page',$3,$4,$5)")
                .bind(root.id.as_str()).bind(&parent).bind(body.len() as i64).bind(height as i32).bind(body).execute(&mut *tx).await.unwrap();
            for (ordinal, child) in children.iter().enumerate() {
                db::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) VALUES($1,$2,$3,$4,$5,$6)")
                    .bind(root.id.as_str()).bind(&parent).bind(ordinal as i32).bind(child.digest.as_str()).bind(child.length.get() as i64).bind(child.chunks.get() as i64).execute(&mut *tx).await.unwrap();
            }
            next.push(PageReference {
                digest: parent.parse().unwrap(),
                length: children
                    .iter()
                    .map(|c| c.length.get())
                    .sum::<u64>()
                    .try_into()
                    .unwrap(),
                chunks: children
                    .iter()
                    .map(|c| c.chunks.get())
                    .sum::<u64>()
                    .try_into()
                    .unwrap(),
            });
        }
        leaves = next;
    }
    let file = FileManifest {
        format: 1,
        profile: CHUNK_PROFILE.into(),
        length: (count * 8).try_into().unwrap(),
        chunks: count.try_into().unwrap(),
        height,
        tree: Some(leaves[0].digest.clone()),
    };
    file.validate().unwrap();
    let body = canonical(&file).unwrap();
    let manifest = digest(&body);
    db::query("INSERT INTO content_objects(root_id,digest,kind,length,height,body) VALUES($1,$2,'file',$3,$4,$5)")
        .bind(root.id.as_str()).bind(&manifest).bind(body.len() as i64).bind(height as i32).bind(body).execute(&mut *tx).await.unwrap();
    db::query("INSERT INTO content_edges(root_id,parent,ordinal,child,logical_length,chunk_count) VALUES($1,$2,0,$3,$4,$5)")
        .bind(root.id.as_str()).bind(&manifest).bind(leaves[0].digest.as_str()).bind((count*8) as i64).bind(count as i64).execute(&mut *tx).await.unwrap();
    db::query("INSERT INTO graph_pins(root_id,id,kind,manifest) VALUES($1,'qualification-pin','checkpoint',$2)")
        .bind(root.id.as_str()).bind(&manifest).execute(&mut *tx).await.unwrap();
    let references:i64=db::query_scalar("SELECT count(*) FROM content_edges e JOIN content_objects c ON c.root_id=e.root_id AND c.digest=e.child WHERE e.root_id=$1 AND c.kind='chunk'")
        .bind(root.id.as_str()).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(references, count as i64);
    db::query("ALTER TABLE content_edges ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .unwrap();
    db::query("UPDATE roots SET graph_serial=graph_serial+1 WHERE id=$1")
        .bind(root.id.as_str())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let _ = w.release().await;
    w = w.catalog().acquire_writer().await.unwrap();
    let fence = w
        .begin_operation(&a, &root.id, OperationKind::Upload)
        .await
        .unwrap();
    let store = Arc::new(MemoryStore::default());
    let stores = BTreeMap::from([("one".into(), store as Arc<dyn ObjectStore>)]);
    let mut engine = TransferEngine::new(w.clone(), stores.clone(), 1, 60).unwrap();
    let (chunk, payload) = first_chunk.unwrap();
    let orphan = digest(b"orphan");
    db::query("INSERT INTO content_objects(root_id,digest,kind,length) VALUES($1,$2,'chunk',6)")
        .bind(root.id.as_str())
        .bind(&orphan)
        .execute(&db)
        .await
        .unwrap();
    w.reserve_transfer(
        &fence,
        &BTreeMap::from([("pool".parse().unwrap(), 14.try_into().unwrap())]),
    )
    .await
    .unwrap();
    for (digest, bytes) in [(&chunk, payload), (&orphan, b"orphan".to_vec())] {
        engine
            .write_copy(
                &fence,
                &digest.parse().unwrap(),
                &"one".parse().unwrap(),
                bytes,
            )
            .await
            .unwrap();
    }
    db::query("INSERT INTO transfer_sessions(id,root_id,operation_id,principal_id,key,request,manifest,state) VALUES('qualification-gc',$1,$2,$3,'gc','{}',$4,'aborted')")
        .bind(root.id.as_str()).bind(fence.id().as_str()).bind(a.principal_id()).bind(&manifest).execute(&db).await.unwrap();
    let seed_ms = seed.elapsed().as_millis();
    let start = Instant::now();
    let baseline = rss();
    let mut peak = baseline;
    let mut passes = 0;
    loop {
        w.renew().await.unwrap();
        engine.heartbeat().await.unwrap();
        let batch = Instant::now();
        engine.tick().await.unwrap();
        passes += 1;
        peak = peak.max(rss());
        assert!(
            batch.elapsed().as_secs() < 25,
            "bounded GC batch exceeded writer lease budget"
        );
        if passes == 5 {
            w.release().await.unwrap();
            w = w.catalog().acquire_writer().await.unwrap();
            engine = TransferEngine::new(w.clone(), stores.clone(), 1, 60).unwrap();
        }
        let state: String = db::query_scalar("SELECT state FROM object_copies WHERE digest=$1")
            .bind(&orphan)
            .fetch_one(&db)
            .await
            .unwrap();
        if state == "deleted" {
            break;
        }
        assert!(passes < 10000, "graph scan failed to converge");
        if passes % 25 == 0 {
            println!(
                "QUALIFY reference_gc passes={passes} elapsed_ms={}",
                start.elapsed().as_millis()
            );
        }
    }
    let state: String = db::query_scalar("SELECT state FROM object_copies WHERE digest=$1")
        .bind(&chunk)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(state, "available");
    let marks: i64 = db::query_scalar("SELECT count(*) FROM graph_marks WHERE root_id=$1")
        .bind(root.id.as_str())
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(marks <= (count.div_ceil(MANIFEST_FANOUT as u64) * 3 + 20) as i64);
    let bytes: i64 = db::query_scalar("SELECT pg_total_relation_size('content_edges')")
        .fetch_one(&db)
        .await
        .unwrap();
    println!(
        "QUALIFY reference_scale references={count} seed_ms={seed_ms} gc_ms={} passes={passes} baseline_rss_kib={baseline} peak_rss_kib={peak} edge_relation_bytes={bytes} marks={marks} pinned_copy_preserved=true orphan_reclaimed=true",
        start.elapsed().as_millis()
    );
}
