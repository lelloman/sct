//! Durable account-owned optical history. No online retention cascades into it.
use crate::{
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    roots::{json, remember, replay},
};
use db::{PgConnection, PgRow};
use sct_protocol::*;
use simple_server::database::postgres as db;

pub(crate) const COLLECTION_SELECT: &str = "SELECT c.*, (extract(epoch FROM c.created_at)*1000)::bigint AS created_ms, (SELECT s.id FROM archive_sets s WHERE s.collection_id=c.id AND s.verified_complete_at IS NOT NULL ORDER BY sequence DESC LIMIT 1) AS last_complete FROM archive_collections c";
pub(crate) fn collection_row(row: &PgRow) -> Result<ArchiveCollection> {
    Ok(ArchiveCollection {
        id: row.get::<String, _>("id").parse()?,
        owner: row.get::<String, _>("owner_id").parse()?,
        label: row.get::<String, _>("label").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        media_capacity_bytes: (row.get::<i64, _>("media_capacity") as u64).try_into()?,
        next_disc_number: (row.get::<i64, _>("next_disc") as u64).try_into()?,
        last_completed_set_id: row
            .get::<Option<String>, _>("last_complete")
            .map(|v| v.parse())
            .transpose()?,
        reconciliation_required: row.get("reconciliation_required"),
        created_at_ms: (row.get::<i64, _>("created_ms") as u64).try_into()?,
    })
}
pub(crate) async fn collection_access(
    db: &mut PgConnection,
    actor: &Actor,
    collection: &ArchiveCollectionId,
    lock: bool,
) -> Result<ArchiveCollection> {
    let admin = auth::administrator(db, actor).await?;
    let row = db::query(&format!(
        "{COLLECTION_SELECT} WHERE c.id=$1{}",
        if lock { " FOR UPDATE OF c" } else { "" }
    ))
    .bind(collection.as_str())
    .fetch_optional(db)
    .await?
    .ok_or(Error(ErrorCode::NotFound))?;
    require(
        admin || row.get::<String, _>("owner_id") == actor.principal_id(),
        ErrorCode::NotFound,
    )?;
    collection_row(&row)
}
pub(crate) const SET_SELECT: &str = "SELECT s.*, (extract(epoch FROM created_at)*1000)::bigint AS created_ms, (extract(epoch FROM expires_at)*1000)::bigint AS expires_ms, (extract(epoch FROM published_at)*1000)::bigint AS published_ms, (extract(epoch FROM verified_complete_at)*1000)::bigint AS complete_ms FROM archive_sets s";
pub(crate) fn set_row(row: &PgRow) -> Result<ArchiveSet> {
    Ok(ArchiveSet {
        id: row.get::<String, _>("id").parse()?,
        collection_id: row.get::<String, _>("collection_id").parse()?,
        sequence: (row.get::<i64, _>("sequence") as u64).try_into()?,
        revision: row.get::<String, _>("revision").parse()?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        selections: serde_json::from_value(row.get("selections"))?,
        media_capacity_bytes: (row.get::<i64, _>("media_capacity") as u64).try_into()?,
        processed_entries: (row.get::<i64, _>("processed_entries") as u64).try_into()?,
        processed_bytes: (row.get::<i64, _>("processed_bytes") as u64).try_into()?,
        plan: row
            .get::<Option<serde_json::Value>, _>("plan")
            .map(serde_json::from_value)
            .transpose()?,
        failure: row
            .get::<Option<String>, _>("failure")
            .map(|s| serde_json::from_value(json(&s).unwrap()))
            .transpose()?,
        created_at_ms: (row.get::<i64, _>("created_ms") as u64).try_into()?,
        expires_at_ms: (row.get::<i64, _>("expires_ms") as u64).try_into()?,
        published_at_ms: row
            .get::<Option<i64>, _>("published_ms")
            .map(|n| (n as u64).try_into())
            .transpose()?,
        verified_complete_at_ms: row
            .get::<Option<i64>, _>("complete_ms")
            .map(|n| (n as u64).try_into())
            .transpose()?,
    })
}
pub(crate) async fn load_set(
    db: &mut PgConnection,
    collection: &ArchiveCollectionId,
    set: &ArchiveSetId,
) -> Result<ArchiveSet> {
    set_row(
        &db::query(&format!("{SET_SELECT} WHERE collection_id=$1 AND id=$2"))
            .bind(collection.as_str())
            .bind(set.as_str())
            .fetch_optional(db)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?,
    )
}
impl Writer {
    pub async fn create_archive_collection(
        &self,
        actor: &Actor,
        request: &CreateArchiveCollectionRequest,
    ) -> Result<ArchiveCollection> {
        require(
            request.media_capacity_bytes.get() > 0,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        auth::administrator(&mut tx, actor).await?;
        let key = request.request_id.as_str().parse()?;
        if let Some(result) =
            replay(&mut tx, actor, "archive-collections", &key, &json(request)?).await?
        {
            return Ok(result);
        }
        db::query("INSERT INTO archive_accounts(principal_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(actor.principal_id())
            .execute(&mut *tx)
            .await?;
        let identity: ArchiveCollectionId = id().parse()?;
        db::query("INSERT INTO archive_collections(id,owner_id,label,revision,media_capacity) VALUES($1,$2,$3,$4,$5)")
            .bind(identity.as_str()).bind(actor.principal_id()).bind(request.label.as_str()).bind(id()).bind(request.media_capacity_bytes.get() as i64).execute(&mut *tx).await?;
        let result = collection_access(&mut tx, actor, &identity, false).await?;
        remember(
            &mut tx,
            actor,
            "archive-collections",
            &key,
            json(request)?,
            json(&result)?,
        )
        .await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "archive.collection.created",
            serde_json::json!({"collection_id":identity}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn create_archive_plan(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        request: &CreateArchivePlanRequest,
    ) -> Result<ArchiveSet> {
        request.validate()?;
        let mut tx = self.transaction().await?;
        let current = collection_access(&mut tx, actor, collection, true).await?;
        crate::archive_capture::lock_capture(&mut tx, actor, &current.owner, &request.selections)
            .await?;
        let scope = format!("archive-set:{}", collection.as_str());
        let key = request.request_id.as_str().parse()?;
        if let Some(result) = replay(&mut tx, actor, &scope, &key, &json(request)?).await? {
            return Ok(result);
        }
        require(
            !current.reconciliation_required,
            ErrorCode::ArchiveReconciliationRequired,
        )?;
        require(
            current.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        let busy:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM archive_sets WHERE collection_id=$1 AND state IN ('capturing','planning','awaiting_confirmation','building'))").bind(collection.as_str()).fetch_one(&mut *tx).await?;
        require(!busy, ErrorCode::ArchiveBusy)?;
        let sequence:Option<i64>=db::query_scalar("UPDATE archive_collections SET next_set=next_set+1,revision=$2 WHERE id=$1 AND next_set<=1000000 RETURNING next_set-1")
            .bind(collection.as_str()).bind(id()).fetch_optional(&mut *tx).await?;
        let sequence = sequence.ok_or(Error(ErrorCode::DestinationCapacity))?;
        let identity: ArchiveSetId = id().parse()?;
        db::query("INSERT INTO archive_sets(id,collection_id,sequence,revision,state,principal_id,credential_id,selections,media_capacity) VALUES($1,$2,$3,$4,'capturing',$5,$6,$7,$8)")
            .bind(identity.as_str()).bind(collection.as_str()).bind(sequence).bind(id()).bind(actor.principal_id()).bind(&actor.credential).bind(json(&request.selections)?).bind(request.media_capacity_bytes.get() as i64).execute(&mut *tx).await?;
        db::query("INSERT INTO archive_sources(set_id,ordinal,root_id,version_id,path) SELECT $1,(ordinality-1)::integer,value->>'root_id',value->>'version_id',value->>'path' FROM jsonb_array_elements($2) WITH ORDINALITY")
            .bind(identity.as_str()).bind(json(&request.selections)?).execute(&mut *tx).await?;
        crate::archive_capture::pin_capture(
            &mut tx,
            &current.owner,
            &identity,
            &request.selections,
        )
        .await?;
        db::query("INSERT INTO archive_jobs(set_id) VALUES($1)")
            .bind(identity.as_str())
            .execute(&mut *tx)
            .await?;
        let result = load_set(&mut tx, collection, &identity).await?;
        remember(&mut tx, actor, &scope, &key, json(request)?, json(&result)?).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "archive.capture.requested",
            serde_json::json!({"collection_id":collection,"set_id":identity}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
}

/// Transactional numbering primitive for the planner. The caller owns the catalog
/// transaction; reservations survive cancellation, so published numbers never recur.
pub async fn reserve_disc_numbers(
    db: &mut PgConnection,
    collection: &ArchiveCollectionId,
    set: &ArchiveSetId,
    count: u64,
) -> Result<(u64, u64)> {
    require(
        (1..=ARCHIVE_MAX_DISCS).contains(&count),
        ErrorCode::InvalidRequest,
    )?;
    let row = db::query(
        "SELECT next_disc,reconciliation_required FROM archive_collections WHERE id=$1 FOR UPDATE",
    )
    .bind(collection.as_str())
    .fetch_optional(&mut *db)
    .await?
    .ok_or(Error(ErrorCode::NotFound))?;
    require(
        !row.get::<bool, _>("reconciliation_required"),
        ErrorCode::ArchiveReconciliationRequired,
    )?;
    let plan=db::query("SELECT first_disc,last_disc,state,frozen_at IS NOT NULL AS frozen FROM archive_sets WHERE id=$1 AND collection_id=$2 FOR UPDATE").bind(set.as_str()).bind(collection.as_str()).fetch_optional(&mut *db).await?.ok_or(Error(ErrorCode::NotFound))?;
    require(
        plan.get::<String, _>("state") == "planning" && !plan.get::<bool, _>("frozen"),
        ErrorCode::RevisionConflict,
    )?;
    if let Some(first) = plan.get::<Option<i64>, _>("first_disc") {
        let last = plan.get::<i64, _>("last_disc");
        require(
            (last - first + 1) as u64 == count,
            ErrorCode::IdempotencyConflict,
        )?;
        return Ok((first as u64, last as u64));
    }
    let first = row.get::<i64, _>("next_disc") as u64;
    let last = first + count - 1;
    require(last <= ARCHIVE_MAX_DISCS, ErrorCode::DestinationCapacity)?;
    db::query("UPDATE archive_collections SET next_disc=$2,revision=$3 WHERE id=$1")
        .bind(collection.as_str())
        .bind((last + 1) as i64)
        .bind(id())
        .execute(&mut *db)
        .await?;
    db::query("UPDATE archive_sets SET first_disc=$2,last_disc=$3,revision=$4 WHERE id=$1")
        .bind(set.as_str())
        .bind(first as i64)
        .bind(last as i64)
        .bind(id())
        .execute(db)
        .await?;
    Ok((first, last))
}
