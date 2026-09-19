use super::*;
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
#[tokio::test]
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
        sqlx::query("ALTER TABLE entries DISABLE TRIGGER USER")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision) SELECT 'scale-'||n,$1,$2,'entry-'||lpad(n::text,7,'0'),'directory','fixture' FROM generate_series($3::bigint,$4::bigint) n")
            .bind(r.id.as_str()).bind(source.id.as_str()).bind(previous+1).bind(count).execute(&mut *tx).await.unwrap();
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE entries ENABLE TRIGGER USER")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE roots SET mutation_serial=mutation_serial+1 WHERE id=$1")
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
                let saved: String = sqlx::query_scalar(
                    "SELECT after_child FROM preview_entry_work WHERE preview_id=$1",
                )
                .bind(preview.id.as_str())
                .fetch_one(&db)
                .await
                .unwrap();
                assert!(!saved.is_empty());
                w.release().await.unwrap();
                w = w.catalog().acquire_writer().await.unwrap();
                let resumed: String = sqlx::query_scalar(
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
                let frontier: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM preview_entry_work WHERE preview_id=$1",
                )
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
        sqlx::query("UPDATE entries SET parent_id=$2,name='source',revision=$3 WHERE id=$1")
            .bind(source.id.as_str())
            .bind(mount.id.as_str())
            .bind(source.revision.as_str())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE entries SET revision=$2 WHERE id=$1")
            .bind(dest.id.as_str())
            .bind(dest.revision.as_str())
            .execute(&db)
            .await
            .unwrap();
        previous = count;
    }
}
