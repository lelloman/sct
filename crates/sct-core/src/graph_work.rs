//! Restartable mark/sweep frontier shared by file and directory roots.
use crate::{
    error::{Result, id},
    operations::OperationFence,
};
use sqlx::{PgConnection, Row};

const SOURCES: [&str; 6] = [
    "SELECT id key,manifest FROM file_versions WHERE root_id=$1 AND NOT retired AND id>$2 ORDER BY id LIMIT 64",
    "SELECT p.id key,v.manifest FROM read_pins p JOIN file_versions v ON v.root_id=p.root_id AND v.id=p.version_id WHERE p.root_id=$1 AND p.expires_at>clock_timestamp() AND p.id>$2 ORDER BY p.id LIMIT 64",
    "SELECT id key,manifest FROM transfer_sessions WHERE root_id=$1 AND state IN ('open','ready') AND admission<>'rejected' AND id>$2 ORDER BY id LIMIT 64",
    "SELECT key,manifest FROM (SELECT d.id||'-base' key,v.manifest FROM tree_drafts d JOIN file_versions v ON v.root_id=d.root_id AND v.id=d.base_version_id WHERE d.root_id=$1 AND d.state IN ('open','ready') UNION ALL SELECT id||'-tree' key,manifest FROM tree_drafts WHERE root_id=$1 AND state IN ('open','ready') AND manifest IS NOT NULL) roots WHERE key>$2 ORDER BY key LIMIT 64",
    "SELECT e.id key,e.manifest FROM draft_entries e JOIN tree_drafts d ON d.id=e.draft_id AND d.root_id=e.root_id WHERE e.root_id=$1 AND d.state IN ('open','ready') AND d.manifest IS NULL AND NOT e.removed AND e.manifest IS NOT NULL AND e.id>$2 ORDER BY e.id LIMIT 64",
    "SELECT id key,manifest FROM graph_pins WHERE root_id=$1 AND (expires_at IS NULL OR expires_at>clock_timestamp()) AND id>$2 ORDER BY id LIMIT 64",
];

async fn mark(db: &mut PgConnection, root: &str, scan: &str, digest: &str) -> Result<()> {
    sqlx::query("INSERT INTO graph_marks(root_id,digest,scan_id) VALUES($1,$2,$3) ON CONFLICT(root_id,digest) DO UPDATE SET scan_id=$3,expanded=false WHERE graph_marks.scan_id<>$3")
        .bind(root).bind(digest).bind(scan).execute(db).await?;
    Ok(())
}

/// Caller holds the root lock and checked the operation fence. False means more
/// bounded work remains; it must never be interpreted as absence of garbage.
pub(crate) async fn advance(db: &mut PgConnection, fence: &OperationFence) -> Result<bool> {
    let generation = sqlx::query("SELECT mutation_serial,graph_serial FROM roots WHERE id=$1")
        .bind(&fence.root)
        .fetch_one(&mut *db)
        .await?;
    let mutation: i64 = generation.get("mutation_serial");
    let graph: i64 = generation.get("graph_serial");
    sqlx::query("INSERT INTO graph_scans(root_id,operation_id,scan_id,mutation_serial,graph_serial) VALUES($1,$2,$3,$4,$5) ON CONFLICT(root_id) DO UPDATE SET operation_id=$2,scan_id=$3,mutation_serial=$4,graph_serial=$5,source=0,after_key='',phase='seeds' WHERE graph_scans.operation_id<>$2 OR graph_scans.mutation_serial<>$4 OR graph_scans.graph_serial<>$5")
        .bind(&fence.root).bind(&fence.id).bind(id()).bind(mutation).bind(graph).execute(&mut *db).await?;
    let row = sqlx::query("SELECT * FROM graph_scans WHERE root_id=$1")
        .bind(&fence.root)
        .fetch_one(&mut *db)
        .await?;
    let scan: String = row.get("scan_id");
    if row.get::<String, _>("phase") == "sweep" {
        return Ok(true);
    }
    let mut source: i32 = row.get("source");
    let mut after: String = row.get("after_key");
    while source < 6 {
        let roots = sqlx::query(SOURCES[source as usize])
            .bind(&fence.root)
            .bind(&after)
            .fetch_all(&mut *db)
            .await?;
        for root in &roots {
            mark(db, &fence.root, &scan, &root.get::<String, _>("manifest")).await?;
        }
        if roots.len() == 64 {
            sqlx::query("UPDATE graph_scans SET source=$2,after_key=$3 WHERE root_id=$1")
                .bind(&fence.root)
                .bind(source)
                .bind(roots.last().unwrap().get::<String, _>("key"))
                .execute(&mut *db)
                .await?;
            return Ok(false);
        }
        source += 1;
        after.clear();
    }
    sqlx::query("UPDATE graph_scans SET source=6,phase='expand',after_key='' WHERE root_id=$1")
        .bind(&fence.root)
        .execute(&mut *db)
        .await?;
    // Process at most 64 immutable nodes. Each adjacency has <=1024 children.
    // Query the next node again so small graphs finish in a single pass.
    for _ in 0..64 {
        let next:Option<String>=sqlx::query_scalar("SELECT digest FROM graph_marks WHERE root_id=$1 AND scan_id=$2 AND NOT expanded ORDER BY digest LIMIT 1")
            .bind(&fence.root).bind(&scan).fetch_optional(&mut *db).await?;
        let Some(next) = next else {
            sqlx::query("UPDATE graph_scans SET phase='sweep' WHERE root_id=$1")
                .bind(&fence.root)
                .execute(&mut *db)
                .await?;
            return Ok(true);
        };
        let children:Vec<String>=sqlx::query_scalar("SELECT child FROM content_edges WHERE root_id=$1 AND parent=$2 ORDER BY ordinal LIMIT 1024")
            .bind(&fence.root).bind(&next).fetch_all(&mut *db).await?;
        for child in children {
            mark(db, &fence.root, &scan, &child).await?;
        }
        sqlx::query(
            "UPDATE graph_marks SET expanded=true WHERE root_id=$1 AND digest=$2 AND scan_id=$3",
        )
        .bind(&fence.root)
        .bind(next)
        .bind(&scan)
        .execute(&mut *db)
        .await?;
    }
    Ok(false)
}
