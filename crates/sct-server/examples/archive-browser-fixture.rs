//! Synthetic historical metadata for real-server browser tests. No ISO/burn claim.
use db::PgPool;
use sct_protocol::*;
use simple_server::database::postgres as db;

#[simple_server::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("SCT_E2E_DATABASE_URL")?;
    if !url.contains("sct_e2e_") {
        return Err("requires disposable browser-test database".into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("expected collection and root IDs".into());
    }
    let collection: ArchiveCollectionId = args[0].parse()?;
    let root: RootId = args[1].parse()?;
    let db = PgPool::connect(&url).await?;
    let mut tx = db.begin().await?;
    let owner: String =
        db::query_scalar("SELECT owner_id FROM archive_collections WHERE id=$1 FOR UPDATE")
            .bind(collection.as_str())
            .fetch_one(&mut *tx)
            .await?;
    let root_owner: String = db::query_scalar("SELECT owner_id FROM roots WHERE id=$1")
        .bind(root.as_str())
        .fetch_one(&mut *tx)
        .await?;
    if owner != "alice" || root_owner != owner {
        return Err("fixture requires Alice's collection and root".into());
    }
    db::query("SET LOCAL sct.archive_import='on'")
        .execute(&mut *tx)
        .await?;
    let selections =
        serde_json::json!([{"root_id":root,"version_id":"archived_version","path":"/"}]);
    let plan = serde_json::json!({"source_files":1,"source_bytes":1,"new_payload_bytes":1,"reused_payload_bytes":0,"new_disc_count":1,"required_earlier_disc_count":0,"index_bytes_per_disc":1024,"bundle_bytes_per_disc":1024,"filesystem_reserve_per_disc":2048,"total_iso_budget_bytes":4096,"bundle_sha256":"a".repeat(64)});
    for n in 1..=22 {
        let set = format!("{}_set_{n:02}", collection.as_str());
        let disc = format!("{}_disc_{n:02}", collection.as_str());
        let hash = format!("{n:064x}");
        db::query("INSERT INTO archive_sets(id,collection_id,sequence,revision,state,principal_id,credential_id,selections,media_capacity,plan,first_disc,last_disc,frozen_at,published_at,verified_complete_at) VALUES($1,$2,$3,'fixture','ready',$4,'historical_credential',$5,25000000000,$6,$3,$3,clock_timestamp(),clock_timestamp(),clock_timestamp())")
            .bind(&set).bind(collection.as_str()).bind(n as i64).bind(&owner).bind(&selections).bind(&plan).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_sources(set_id,ordinal,root_id,version_id,path,kind) VALUES($1,0,$2,'archived_version','/','directory')").bind(&set).bind(root.as_str()).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_entries(set_id,path,kind,root_id,version_id,length) VALUES($1,$2,'file',$3,'archived_version',1)").bind(&set).bind(format!("/historical-{n}.txt")).bind(root.as_str()).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_objects VALUES($1,$2,$3,1)")
            .bind(&set)
            .bind(root.as_str())
            .bind(&hash)
            .execute(&mut *tx)
            .await?;
        db::query("INSERT INTO archive_discs(id,collection_id,set_id,number,index_sha256,bundle_sha256,inventory_sha256,payload_bytes,object_count,planned_iso_bytes) VALUES($1,$2,$3,$4,$5,$5,$5,1,1,4096)").bind(&disc).bind(collection.as_str()).bind(&set).bind(n as i64).bind(&hash).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_placements VALUES($1,$2,$3,$4,1,$5)")
            .bind(&set)
            .bind(collection.as_str())
            .bind(root.as_str())
            .bind(&hash)
            .bind(&disc)
            .execute(&mut *tx)
            .await?;
        db::query("INSERT INTO archive_artifacts(id,disc_id,state,sha256,length,expires_at) VALUES($1,$2,'expired',$3,4096,clock_timestamp()-interval '1 day')").bind(format!("{disc}_iso")).bind(&disc).bind(&hash).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_copies(id,disc_id,label,location,revision,health) VALUES($1,$2,$3,'Shelf A','fixture',$4)").bind(format!("{disc}_copy")).bind(&disc).bind(format!("Copy {n}")).bind(if n==2 {"missing"} else {"read_back_verified"}).execute(&mut *tx).await?;
    }
    let latest = format!("{}_set_22", collection.as_str());
    db::query("INSERT INTO archive_objects VALUES($1,$2,$3,1)")
        .bind(&latest)
        .bind(root.as_str())
        .bind(format!("{:064x}", 1))
        .execute(&mut *tx)
        .await?;
    db::query("INSERT INTO archive_placements VALUES($1,$2,$3,$4,1,$5)")
        .bind(&latest)
        .bind(collection.as_str())
        .bind(root.as_str())
        .bind(format!("{:064x}", 1))
        .bind(format!("{}_disc_01", collection.as_str()))
        .execute(&mut *tx)
        .await?;
    db::query("UPDATE archive_collections SET next_disc=23,next_set=23 WHERE id=$1")
        .bind(collection.as_str())
        .execute(&mut *tx)
        .await?;
    let count = db::query("SELECT count(*) FROM archive_sets WHERE collection_id=$1")
        .bind(collection.as_str())
        .fetch_one(&mut *tx)
        .await?
        .get::<i64, _>(0);
    if count != 22 {
        return Err("fixture collection was not empty".into());
    }
    tx.commit().await?;
    Ok(())
}
