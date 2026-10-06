//! Restartable, root-scoped optical packing and canonical index generation.
use crate::{
    archive_catalog::*,
    archive_management::source_access,
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    roots::json,
};
use db::PgConnection;
use sct_protocol::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use simple_server::database::postgres as db;

const BATCH: i64 = 128;
const ISO_BASE: u64 = 2 * 1024 * 1024;
fn sector(n: u64) -> u64 {
    n.div_ceil(2048) * 2048
}
fn object_cost(n: u64) -> u64 {
    sector(n) + 8192
}
fn disc_id(set: &str, number: u64) -> String {
    format!("{set}_{number:07}")
}
fn bytes(record: &ArchiveRecord) -> Result<Vec<u8>> {
    record
        .validate()
        .map_err(|_| Error(ErrorCode::ArchiveMetadataTooLarge))?;
    let mut line = canonical(record)?;
    line.push(b'\n');
    Ok(line)
}
fn object(row: &db::PgRow) -> Result<ArchiveObject> {
    Ok(ArchiveObject {
        root_id: row.get::<String, _>("root_id").parse()?,
        digest: row.get::<String, _>("digest").parse()?,
        length: (row.get::<i64, _>("length") as u64).try_into()?,
    })
}
async fn record(
    db: &mut PgConnection,
    set: &str,
    phase: i32,
    key1: &str,
    key2: &str,
    position: i64,
    value: &ArchiveRecord,
) -> Result<u64> {
    let line = bytes(value)?;
    let length = line.len() as u64;
    let inserted=db::query("INSERT INTO archive_index_records(set_id,phase,key1,key2,position,line) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
        .bind(set).bind(phase).bind(key1).bind(key2).bind(position).bind(line).execute(db).await?.rows_affected();
    Ok(length * inserted)
}
#[derive(Default, Serialize, Deserialize)]
struct Plan {
    phase: u32,
    path: String,
    offset: i64,
    ordinal: i64,
    base_bytes: u64,
    files: u64,
    object_count: u64,
    new_bytes: u64,
    reused_bytes: u64,
    placement_bytes: u64,
    old_disc_bytes: u64,
    old_discs: u64,
    bound: u64,
    packing_disc: u64,
    packing_bytes: u64,
    passes: u32,
    first: u64,
    last: u64,
    metadata_budget: u64,
    index_bytes: u64,
    index_records: u64,
    index_hash: Option<ChunkDigest>,
    disc_cursor: u64,
    total_budget: u64,
}
fn header(set: &ArchiveSet, first: u64, last: u64) -> ArchiveRecord {
    ArchiveRecord::Snapshot {
        format: ARCHIVE_FORMAT,
        collection_id: set.collection_id.as_str().to_owned(),
        set_id: set.id.as_str().to_owned(),
        set_sequence: set.sequence.get(),
        first_disc: first,
        last_disc: last,
        sources: set
            .selections
            .iter()
            .map(|s| ArchiveSource {
                root_id: s.root_id.clone(),
                version_id: s.version_id.clone(),
                path: s.path.clone(),
            })
            .collect(),
    }
}
fn bundle_cost(bundle: &ArchiveBundle) -> Result<(u64, u64)> {
    bundle.validate()?;
    let manifest = canonical(bundle)?.len() as u64;
    let logical = manifest + bundle.files.iter().map(|f| f.length.get()).sum::<u64>();
    let physical = sector(manifest)
        + bundle
            .files
            .iter()
            .map(|f| sector(f.length.get()) + 8192 + 4096 * f.path.split('/').count() as u64)
            .sum::<u64>();
    Ok((logical, physical))
}
fn budget(set: &ArchiveSet, plan: &Plan, bundle: &ArchiveBundle) -> Result<u64> {
    let disc = ArchiveRecord::Disc {
        disc_id: disc_id(set.id.as_str(), ARCHIVE_MAX_DISCS),
        number: ARCHIVE_MAX_DISCS,
        set_sequence: set.sequence.get(),
    };
    let index = bytes(&header(set, ARCHIVE_MAX_DISCS, ARCHIVE_MAX_DISCS))?.len() as u64
        + plan.base_bytes
        + plan.placement_bytes
        + plan.old_disc_bytes
        + plan.bound * bytes(&disc)?.len() as u64;
    require(
        index <= ARCHIVE_MAX_INDEX_BYTES,
        ErrorCode::ArchiveMetadataTooLarge,
    )?;
    Ok(ISO_BASE + bundle_cost(bundle)?.1 + sector(index) + 8192)
}
async fn reusable(
    db: &mut PgConnection,
    actor: &Actor,
    set: &ArchiveSet,
    obj: &ArchiveObject,
) -> Result<Option<(String, u64, u64)>> {
    let admin = auth::administrator(db, actor).await?;
    let row=db::query("SELECT d.id,d.number,s.sequence,s.selections FROM archive_discs d JOIN archive_sets s ON s.id=d.set_id JOIN archive_placements p ON p.disc_id=d.id AND p.set_id=d.set_id WHERE d.collection_id=$1 AND s.sequence<$2 AND s.verified_complete_at IS NOT NULL AND p.root_id=$3 AND p.digest=$4 AND p.length=$5 AND EXISTS(SELECT 1 FROM archive_copies c WHERE c.disc_id=d.id AND c.health='read_back_verified') AND NOT EXISTS(SELECT 1 FROM archive_sources src LEFT JOIN roots r ON r.id=src.root_id WHERE src.set_id=s.id AND (r.id IS NULL OR NOT ($6 OR r.owner_id=$7 OR EXISTS(SELECT 1 FROM grants g WHERE g.root_id=r.id AND g.principal_id=$7 AND g.capabilities @> '[\"list_read\"]'::jsonb)))) ORDER BY d.number LIMIT 1")
        .bind(set.collection_id.as_str()).bind(set.sequence.get() as i64).bind(obj.root_id.as_str()).bind(obj.digest.as_str()).bind(obj.length.get() as i64).bind(admin).bind(actor.principal_id()).fetch_optional(&mut *db).await?;
    if let Some(row) = row {
        let sources: Vec<ArchiveSelection> = serde_json::from_value(row.get("selections"))?;
        source_access(db, actor, &sources).await?;
        Ok(Some((
            row.get("id"),
            row.get::<i64, _>("number") as u64,
            row.get::<i64, _>("sequence") as u64,
        )))
    } else {
        Ok(None)
    }
}
impl Writer {
    /// Advance one persistent phase/page. Bundle content is verified by the export
    /// worker before calling this; its canonical identity cannot change on resume.
    pub async fn advance_archive_plan(
        &self,
        identity: &ArchiveSetId,
        bundle: &ArchiveBundle,
    ) -> Result<bool> {
        bundle_cost(bundle)?;
        if !self.authorize_archive_work(identity).await? {
            return Ok(false);
        }
        let mut tx = self.transaction().await?;
        let col:String=db::query_scalar("SELECT c.id FROM archive_collections c JOIN archive_sets s ON s.collection_id=c.id WHERE s.id=$1 FOR UPDATE OF c").bind(identity.as_str()).fetch_one(&mut *tx).await?;
        let row=db::query("SELECT principal_id,credential_id,state,expires_at>clock_timestamp() AS live FROM archive_sets WHERE id=$1 FOR UPDATE").bind(identity.as_str()).fetch_one(&mut *tx).await?;
        if row.get::<String, _>("state") != "planning" {
            return Ok(false);
        }
        require(row.get("live"), ErrorCode::ArchiveExpired)?;
        let actor = Actor {
            principal: row.get("principal_id"),
            credential: row.get("credential_id"),
        };
        let set = load_set(&mut tx, &col.parse()?, identity).await?;
        source_access(&mut tx, &actor, &set.selections).await?;
        db::query("INSERT INTO archive_plan_state(set_id,bundle,cursor) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(identity.as_str()).bind(json(bundle)?).bind(json(&Plan::default())?).execute(&mut *tx).await?;
        let stored = db::query("SELECT bundle,cursor FROM archive_plan_state WHERE set_id=$1")
            .bind(identity.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(
            stored.get::<serde_json::Value, _>("bundle") == json(bundle)?,
            ErrorCode::IntegrityFailure,
        )?;
        let mut p: Plan = serde_json::from_value(stored.get("cursor"))?;
        let sid = identity.as_str();
        match p.phase {
            0 => {
                let rows=db::query("SELECT * FROM archive_entries WHERE set_id=$1 AND path COLLATE \"C\">$2 COLLATE \"C\" ORDER BY path COLLATE \"C\" LIMIT $3").bind(sid).bind(&p.path).bind(BATCH).fetch_all(&mut *tx).await?;
                for row in &rows {
                    let path: String = row.get("path");
                    let value = if row.get::<String, _>("kind") == "directory" {
                        ArchiveRecord::Directory {
                            path: path.parse()?,
                        }
                    } else {
                        p.files += 1;
                        ArchiveRecord::File {
                            path: path.parse()?,
                            root_id: row.get::<String, _>("root_id").parse()?,
                            version_id: row.get::<String, _>("version_id").parse()?,
                            length: (row.get::<i64, _>("length") as u64).try_into()?,
                        }
                    };
                    p.base_bytes += record(&mut tx, sid, 2, &path, "", 0, &value).await?;
                    p.path = path;
                }
                if rows.len() < BATCH as usize {
                    p.phase = 1;
                    p.path.clear();
                    p.offset = -1;
                }
            }
            1 => {
                let rows=db::query("SELECT * FROM archive_extents WHERE set_id=$1 AND (path COLLATE \"C\",offset_bytes)>($2 COLLATE \"C\",$3) ORDER BY path COLLATE \"C\",offset_bytes LIMIT $4").bind(sid).bind(&p.path).bind(p.offset).bind(BATCH).fetch_all(&mut *tx).await?;
                for row in &rows {
                    let path: String = row.get("path");
                    let offset: i64 = row.get("offset_bytes");
                    let obj = object(row)?;
                    p.base_bytes += record(
                        &mut tx,
                        sid,
                        3,
                        &path,
                        "",
                        offset,
                        &ArchiveRecord::Extent {
                            path: path.parse()?,
                            offset: (offset as u64).try_into()?,
                            object: obj.clone(),
                        },
                    )
                    .await?;
                    p.object_count+=db::query("INSERT INTO archive_plan_objects(set_id,root_id,digest,length,ordinal) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(sid).bind(obj.root_id.as_str()).bind(obj.digest.as_str()).bind(obj.length.get() as i64).bind(p.ordinal).execute(&mut *tx).await?.rows_affected();
                    p.ordinal += 1;
                    p.path = path;
                    p.offset = offset;
                }
                if rows.len() < BATCH as usize {
                    p.phase = 2;
                    p.ordinal = -1;
                }
            }
            2 => {
                let rows=db::query("SELECT * FROM archive_plan_objects WHERE set_id=$1 AND ordinal>$2 ORDER BY ordinal LIMIT $3").bind(sid).bind(p.ordinal).bind(BATCH).fetch_all(&mut *tx).await?;
                for row in &rows {
                    let obj = object(row)?;
                    let placement = if let Some((disc, number, sequence)) =
                        reusable(&mut tx, &actor, &set, &obj).await?
                    {
                        db::query("UPDATE archive_plan_objects SET old_disc=$4 WHERE set_id=$1 AND root_id=$2 AND digest=$3").bind(sid).bind(obj.root_id.as_str()).bind(obj.digest.as_str()).bind(&disc).execute(&mut *tx).await?;
                        let added = record(
                            &mut tx,
                            sid,
                            1,
                            &format!("{number:07}"),
                            "",
                            0,
                            &ArchiveRecord::Disc {
                                disc_id: disc.clone(),
                                number,
                                set_sequence: sequence,
                            },
                        )
                        .await?;
                        if added > 0 {
                            p.old_discs += 1;
                            p.old_disc_bytes += added;
                        }
                        p.reused_bytes += obj.length.get();
                        disc
                    } else {
                        p.new_bytes += obj.length.get();
                        disc_id(sid, ARCHIVE_MAX_DISCS)
                    };
                    p.placement_bytes += bytes(&ArchiveRecord::Placement {
                        object: obj,
                        disc_id: placement,
                    })?
                    .len() as u64;
                    p.ordinal = row.get("ordinal");
                }
                if rows.len() < BATCH as usize {
                    p.phase = 3;
                    p.ordinal = -1;
                    p.bound = 1;
                    p.packing_disc = 1;
                    p.metadata_budget = budget(&set, &p, bundle)?;
                }
            }
            3 => {
                require(
                    p.metadata_budget < set.media_capacity_bytes.get(),
                    ErrorCode::ArchiveMetadataTooLarge,
                )?;
                let capacity = set.media_capacity_bytes.get() - p.metadata_budget;
                let rows=db::query("SELECT * FROM archive_plan_objects WHERE set_id=$1 AND ordinal>$2 AND old_disc IS NULL ORDER BY ordinal LIMIT $3").bind(sid).bind(p.ordinal).bind(BATCH).fetch_all(&mut *tx).await?;
                for row in &rows {
                    let obj = object(row)?;
                    let cost = object_cost(obj.length.get());
                    require(cost <= capacity, ErrorCode::ArchiveMetadataTooLarge)?;
                    if p.packing_bytes + cost > capacity {
                        p.packing_disc += 1;
                        p.packing_bytes = 0;
                    }
                    require(
                        p.packing_disc <= ARCHIVE_MAX_DISCS,
                        ErrorCode::ArchiveMetadataTooLarge,
                    )?;
                    db::query("UPDATE archive_plan_objects SET new_disc=$3 WHERE set_id=$1 AND ordinal=$2").bind(sid).bind(row.get::<i64,_>("ordinal")).bind(p.packing_disc as i64).execute(&mut *tx).await?;
                    p.packing_bytes += cost;
                    p.ordinal = row.get("ordinal");
                }
                if rows.len() < BATCH as usize {
                    if p.packing_disc > p.bound {
                        p.passes += 1;
                        require(p.passes <= 32, ErrorCode::ArchiveMetadataTooLarge)?;
                        p.bound = p.packing_disc;
                        p.metadata_budget = budget(&set, &p, bundle)?;
                        p.packing_disc = 1;
                        p.packing_bytes = 0;
                        p.ordinal = -1;
                    } else {
                        (p.first, p.last) = reserve_disc_numbers(
                            &mut tx,
                            &set.collection_id,
                            identity,
                            p.packing_disc,
                        )
                        .await?;
                        record(&mut tx, sid, 0, "", "", 0, &header(&set, p.first, p.last)).await?;
                        p.phase = 4;
                        p.disc_cursor = 0;
                    }
                }
            }
            4 => {
                let end = (p.disc_cursor + BATCH as u64).min(p.packing_disc);
                for local in p.disc_cursor + 1..=end {
                    let number = p.first + local - 1;
                    let disc = disc_id(sid, number);
                    db::query("INSERT INTO archive_plan_discs(set_id,number) VALUES($1,$2)")
                        .bind(sid)
                        .bind(local as i64)
                        .execute(&mut *tx)
                        .await?;
                    db::query("INSERT INTO archive_discs(id,collection_id,set_id,number,index_sha256,bundle_sha256,inventory_sha256,payload_bytes,object_count,planned_iso_bytes) VALUES($1,$2,$3,$4,$5,$6,$5,0,0,$7)").bind(&disc).bind(set.collection_id.as_str()).bind(sid).bind(number as i64).bind("0".repeat(64)).bind(digest(&canonical(bundle)?)).bind(p.metadata_budget as i64).execute(&mut *tx).await?;
                    record(
                        &mut tx,
                        sid,
                        1,
                        &format!("{number:07}"),
                        "",
                        0,
                        &ArchiveRecord::Disc {
                            disc_id: disc,
                            number,
                            set_sequence: set.sequence.get(),
                        },
                    )
                    .await?;
                }
                p.disc_cursor = end;
                if end == p.packing_disc {
                    p.phase = 5;
                    p.ordinal = -1;
                }
            }
            5 => {
                let rows=db::query("SELECT * FROM archive_plan_objects WHERE set_id=$1 AND ordinal>$2 ORDER BY ordinal LIMIT $3").bind(sid).bind(p.ordinal).bind(BATCH).fetch_all(&mut *tx).await?;
                for row in &rows {
                    let obj = object(row)?;
                    let disc = if let Some(old) = row.get::<Option<String>, _>("old_disc") {
                        old
                    } else {
                        let local = row.get::<i64, _>("new_disc");
                        db::query("UPDATE archive_plan_discs SET payload_bytes=payload_bytes+$3,object_count=object_count+1,occupied_bytes=occupied_bytes+$4 WHERE set_id=$1 AND number=$2").bind(sid).bind(local).bind(obj.length.get() as i64).bind(object_cost(obj.length.get()) as i64).execute(&mut *tx).await?;
                        disc_id(sid, p.first + local as u64 - 1)
                    };
                    db::query("INSERT INTO archive_placements(set_id,collection_id,root_id,digest,length,disc_id) VALUES($1,$2,$3,$4,$5,$6)").bind(sid).bind(set.collection_id.as_str()).bind(obj.root_id.as_str()).bind(obj.digest.as_str()).bind(obj.length.get() as i64).bind(&disc).execute(&mut *tx).await?;
                    record(
                        &mut tx,
                        sid,
                        4,
                        obj.root_id.as_str(),
                        obj.digest.as_str(),
                        0,
                        &ArchiveRecord::Placement {
                            object: obj.clone(),
                            disc_id: disc,
                        },
                    )
                    .await?;
                    p.ordinal = row.get("ordinal");
                }
                if rows.len() < BATCH as usize {
                    p.phase = 6;
                }
            }
            6 => {
                // Hash outside the row-locked transaction. All input rows have been
                // committed, and every page rechecks the active set/authority.
                drop(tx);
                let (hash, length, count) = self.hash_archive_index(identity).await?;
                let mut tx = self.transaction().await?;
                check_resume(&mut tx, identity, &p).await?;
                p.index_hash = Some(hash);
                p.index_bytes = length;
                p.index_records = count;
                p.phase = 7;
                p.disc_cursor = 0;
                db::query("UPDATE archive_plan_state SET cursor=$2 WHERE set_id=$1")
                    .bind(sid)
                    .bind(json(&p)?)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                return Ok(true);
            }
            7 => {
                // Per-disc inventories are hashed in the same canonical root/digest
                // order as portable placements, independently of file-local packing.
                if p.disc_cursor < p.packing_disc {
                    let local = p.disc_cursor + 1;
                    let disc = disc_id(sid, p.first + local - 1);
                    drop(tx);
                    let hash = self
                        .hash_archive_inventory(identity, &disc.parse()?)
                        .await?;
                    let mut tx = self.transaction().await?;
                    check_resume(&mut tx, identity, &p).await?;
                    let row =
                        db::query("SELECT * FROM archive_plan_discs WHERE set_id=$1 AND number=$2")
                            .bind(sid)
                            .bind(local as i64)
                            .fetch_one(&mut *tx)
                            .await?;
                    let planned = p.metadata_budget + row.get::<i64, _>("occupied_bytes") as u64;
                    require(
                        planned <= set.media_capacity_bytes.get(),
                        ErrorCode::ArchiveMetadataTooLarge,
                    )?;
                    db::query("UPDATE archive_discs SET index_sha256=$2,inventory_sha256=$3,payload_bytes=$4,object_count=$5,planned_iso_bytes=$6 WHERE id=$1").bind(&disc).bind(p.index_hash.as_ref().ok_or(Error(ErrorCode::InternalError))?.as_str()).bind(hash.as_str()).bind(row.get::<i64,_>("payload_bytes")).bind(row.get::<i64,_>("object_count")).bind(planned as i64).execute(&mut *tx).await?;
                    p.disc_cursor = local;
                    p.total_budget += planned;
                    db::query("UPDATE archive_plan_state SET cursor=$2 WHERE set_id=$1")
                        .bind(sid)
                        .bind(json(&p)?)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    return Ok(true);
                }
                validate_reuse(&mut tx, &actor, identity).await?;
                let summary = ArchivePlanSummary {
                    source_files: p.files.try_into()?,
                    source_bytes: set.processed_bytes,
                    new_payload_bytes: p.new_bytes.try_into()?,
                    reused_payload_bytes: p.reused_bytes.try_into()?,
                    new_disc_count: p.packing_disc.try_into()?,
                    required_earlier_disc_count: p.old_discs.try_into()?,
                    index_bytes_per_disc: p.index_bytes.try_into()?,
                    bundle_bytes_per_disc: bundle_cost(bundle)?.0.try_into()?,
                    filesystem_reserve_per_disc: (p.metadata_budget
                        - p.index_bytes
                        - bundle_cost(bundle)?.0)
                        .try_into()?,
                    total_iso_budget_bytes: p.total_budget.try_into()?,
                    bundle_sha256: digest(&canonical(bundle)?).parse()?,
                };
                db::query("UPDATE archive_sets SET state='awaiting_confirmation',plan=$2,revision=$3 WHERE id=$1").bind(sid).bind(json(&summary)?).bind(id()).execute(&mut *tx).await?;
                p.phase = 8;
            }
            _ => return Ok(false),
        }
        db::query("UPDATE archive_plan_state SET cursor=$2 WHERE set_id=$1")
            .bind(sid)
            .bind(json(&p)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveIndexCursor {
    pub phase: i32,
    pub key1: String,
    pub key2: String,
    pub position: i64,
}
impl Default for ArchiveIndexCursor {
    fn default() -> Self {
        Self {
            phase: -1,
            key1: String::new(),
            key2: String::new(),
            position: 0,
        }
    }
}
impl Writer {
    /// Bounded canonical JSONL page for the ISO worker. Cursor columns use byte
    /// collation so Unicode ordering is identical to the standalone reader.
    pub async fn archive_index_page(
        &self,
        set: &ArchiveSetId,
        cursor: &ArchiveIndexCursor,
    ) -> Result<Vec<(ArchiveIndexCursor, Vec<u8>)>> {
        require(
            self.authorize_archive_work(set).await?,
            ErrorCode::ArchiveExpired,
        )?;
        let mut tx = self.transaction().await?;
        let live:bool=db::query_scalar("SELECT state IN ('planning','awaiting_confirmation','building') AND expires_at>clock_timestamp() FROM archive_sets WHERE id=$1").bind(set.as_str()).fetch_one(&mut *tx).await?;
        require(live, ErrorCode::ArchiveExpired)?;
        let rows=db::query("SELECT phase,key1,key2,position,line FROM archive_index_records WHERE set_id=$1 AND (phase,key1,key2,position)>($2,$3 COLLATE \"C\",$4 COLLATE \"C\",$5) ORDER BY phase,key1,key2,position LIMIT $6").bind(set.as_str()).bind(cursor.phase).bind(&cursor.key1).bind(&cursor.key2).bind(cursor.position).bind(BATCH).fetch_all(&mut *tx).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    ArchiveIndexCursor {
                        phase: r.get("phase"),
                        key1: r.get("key1"),
                        key2: r.get("key2"),
                        position: r.get("position"),
                    },
                    r.get("line"),
                )
            })
            .collect())
    }
    async fn hash_archive_index(&self, set: &ArchiveSetId) -> Result<(ChunkDigest, u64, u64)> {
        let mut hash = Sha256::new();
        let mut cursor = ArchiveIndexCursor::default();
        let mut length = 0;
        let mut count = 0;
        loop {
            let rows = self.archive_index_page(set, &cursor).await?;
            if rows.is_empty() {
                break;
            }
            for (next, line) in rows {
                hash.update(&line);
                length += line.len() as u64;
                count += 1;
                cursor = next;
            }
            require(
                length <= ARCHIVE_MAX_INDEX_BYTES && count <= ARCHIVE_MAX_RECORDS,
                ErrorCode::ArchiveMetadataTooLarge,
            )?;
        }
        Ok((format!("{:x}", hash.finalize()).parse()?, length, count))
    }
    async fn hash_archive_inventory(
        &self,
        set: &ArchiveSetId,
        disc: &ArchiveDiscId,
    ) -> Result<ChunkDigest> {
        let mut hash = Sha256::new();
        let mut root = String::new();
        let mut digest_cursor = String::new();
        loop {
            require(
                self.authorize_archive_work(set).await?,
                ErrorCode::ArchiveExpired,
            )?;
            let mut tx = self.transaction().await?;
            let rows=db::query("SELECT root_id,digest,length FROM archive_placements WHERE set_id=$1 AND disc_id=$2 AND (root_id COLLATE \"C\",digest COLLATE \"C\")>($3 COLLATE \"C\",$4 COLLATE \"C\") ORDER BY root_id COLLATE \"C\",digest COLLATE \"C\" LIMIT $5").bind(set.as_str()).bind(disc.as_str()).bind(&root).bind(&digest_cursor).bind(BATCH).fetch_all(&mut *tx).await?;
            drop(tx);
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let obj = object(&row)?;
                hash.update(canonical(&obj)?);
                hash.update(b"\n");
                root = obj.root_id.as_str().to_owned();
                digest_cursor = obj.digest.as_str().to_owned();
            }
        }
        Ok(format!("{:x}", hash.finalize()).parse()?)
    }
    pub async fn archive_disc_descriptor(
        &self,
        set: &ArchiveSetId,
        disc: &ArchiveDiscId,
    ) -> Result<ArchiveDiscDescriptor> {
        require(
            self.authorize_archive_work(set).await?,
            ErrorCode::ArchiveExpired,
        )?;
        let mut tx = self.transaction().await?;
        let row=db::query("SELECT d.*,s.sequence,s.first_disc,s.last_disc,p.cursor FROM archive_discs d JOIN archive_sets s ON s.id=d.set_id JOIN archive_plan_state p ON p.set_id=s.id WHERE d.id=$1 AND d.set_id=$2").bind(disc.as_str()).bind(set.as_str()).fetch_one(&mut *tx).await?;
        let plan: Plan = serde_json::from_value(row.get("cursor"))?;
        require(plan.phase >= 8, ErrorCode::RevisionConflict)?;
        let descriptor = ArchiveDiscDescriptor {
            format: ARCHIVE_FORMAT,
            collection_id: row.get("collection_id"),
            set_id: set.as_str().to_owned(),
            set_sequence: row.get::<i64, _>("sequence") as u64,
            disc_id: disc.as_str().to_owned(),
            number: row.get::<i64, _>("number") as u64,
            first_disc: row.get::<i64, _>("first_disc") as u64,
            last_disc: row.get::<i64, _>("last_disc") as u64,
            index_sha256: row.get::<String, _>("index_sha256").parse()?,
            index_bytes: plan.index_bytes,
            index_records: plan.index_records,
            bundle_sha256: row.get::<String, _>("bundle_sha256").parse()?,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

async fn check_resume(db: &mut PgConnection, set: &ArchiveSetId, expected: &Plan) -> Result<()> {
    db::query("SELECT c.id FROM archive_collections c JOIN archive_sets s ON s.collection_id=c.id WHERE s.id=$1 FOR UPDATE OF c").bind(set.as_str()).fetch_one(&mut *db).await?;
    let row=db::query("SELECT state,principal_id,credential_id,selections,expires_at>clock_timestamp() AS live FROM archive_sets WHERE id=$1 FOR UPDATE").bind(set.as_str()).fetch_one(&mut *db).await?;
    require(
        row.get::<String, _>("state") == "planning" && row.get::<bool, _>("live"),
        ErrorCode::ArchiveExpired,
    )?;
    let actor = Actor {
        principal: row.get("principal_id"),
        credential: row.get("credential_id"),
    };
    source_access(
        db,
        &actor,
        &serde_json::from_value::<Vec<ArchiveSelection>>(row.get("selections"))?,
    )
    .await?;
    let current: serde_json::Value =
        db::query_scalar("SELECT cursor FROM archive_plan_state WHERE set_id=$1")
            .bind(set.as_str())
            .fetch_one(db)
            .await?;
    require(current == json(expected)?, ErrorCode::ArchiveBusy)
}

/// Rechecked at preview/confirmation and by the ISO publisher; a changed physical
/// observation never silently turns into an assumed-readable dependency.
pub(crate) async fn validate_reuse(
    db: &mut PgConnection,
    actor: &Actor,
    set: &ArchiveSetId,
) -> Result<()> {
    let admin = auth::administrator(db, actor).await?;
    let available:bool=db::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM archive_placements p JOIN archive_discs d ON d.id=p.disc_id JOIN archive_sets s ON s.id=d.set_id WHERE p.set_id=$1 AND d.set_id<>$1 AND (s.verified_complete_at IS NULL OR NOT EXISTS(SELECT 1 FROM archive_copies c WHERE c.disc_id=d.id AND c.health='read_back_verified')))").bind(set.as_str()).fetch_one(&mut *db).await?;
    require(available, ErrorCode::ArchiveMediaUnavailable)?;
    let allowed:bool=db::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM archive_placements p JOIN archive_discs d ON d.id=p.disc_id JOIN archive_sources src ON src.set_id=d.set_id LEFT JOIN roots r ON r.id=src.root_id WHERE p.set_id=$1 AND d.set_id<>$1 AND (r.id IS NULL OR NOT ($2 OR r.owner_id=$3 OR EXISTS(SELECT 1 FROM grants g WHERE g.root_id=r.id AND g.principal_id=$3 AND g.capabilities @> '[\"list_read\"]'::jsonb))))").bind(set.as_str()).bind(admin).bind(actor.principal_id()).fetch_one(db).await?;
    require(allowed, ErrorCode::Forbidden)
}
