//! Physical-copy observations never rewrite logical media or past completion.
use crate::{
    archive_catalog::collection_access,
    archive_management::disc_access,
    auth::Actor,
    error::{Error, Result, id, require},
    lease::Writer,
    paging,
    roots::{json, remember, replay},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

const COPY_SELECT: &str = "SELECT c.*,(extract(epoch FROM c.reported_burned_at)*1000)::bigint AS burned_ms FROM archive_copies c";
fn copy_row(row: &PgRow) -> Result<ArchivePhysicalCopy> {
    Ok(ArchivePhysicalCopy {
        id: row.get::<String, _>("id").parse()?,
        disc_id: row.get::<String, _>("disc_id").parse()?,
        label: row.get::<String, _>("label").parse()?,
        location: row.get("location"),
        revision: row.get::<String, _>("revision").parse()?,
        reported_burned_at_ms: row
            .get::<Option<i64>, _>("burned_ms")
            .map(|n| (n as u64).try_into())
            .transpose()?,
        health: serde_json::from_value(json(&row.get::<String, _>("health"))?)?,
        latest_report_id: row
            .get::<Option<String>, _>("latest_report_id")
            .map(|s| s.parse())
            .transpose()?,
    })
}
async fn load_copy(db: &mut PgConnection, copy: &ArchiveCopyId) -> Result<ArchivePhysicalCopy> {
    copy_row(
        &sqlx::query(&format!("{COPY_SELECT} WHERE c.id=$1"))
            .bind(copy.as_str())
            .fetch_optional(db)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?,
    )
}
async fn copy_access(
    db: &mut PgConnection,
    actor: &Actor,
    collection: &ArchiveCollectionId,
    copy: &ArchiveCopyId,
    lock: bool,
) -> Result<ArchivePhysicalCopy> {
    collection_access(db, actor, collection, lock).await?;
    let disc: String = sqlx::query_scalar("SELECT c.disc_id FROM archive_copies c JOIN archive_discs d ON d.id=c.disc_id WHERE c.id=$1 AND d.collection_id=$2").bind(copy.as_str()).bind(collection.as_str()).fetch_optional(&mut *db).await?.ok_or(Error(ErrorCode::NotFound))?;
    disc_access(db, actor, collection, &disc.parse()?).await?;
    load_copy(db, copy).await
}
fn location(value: &str) -> Result<()> {
    require(
        value.len() <= 1024 && !value.chars().any(char::is_control),
        ErrorCode::InvalidRequest,
    )
}
async fn event(
    db: &mut PgConnection,
    actor: &Actor,
    copy: &ArchivePhysicalCopy,
    kind: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO archive_copy_events(id,copy_id,principal_id,kind,observation) VALUES($1,$2,$3,$4,$5)").bind(id()).bind(copy.id.as_str()).bind(actor.principal_id()).bind(kind).bind(json(copy)?).execute(&mut *db).await?;
    crate::management::audit(
        db,
        actor,
        None,
        "archive.copy.observed",
        serde_json::json!({"copy_id":copy.id,"kind":kind}),
    )
    .await
}
async fn after(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    request: &ListRequest,
) -> Result<String> {
    match &request.cursor {
        Some(cursor) => paging::consume(db, actor, scope, cursor, &json(&request.limit)?).await,
        None => Ok(String::new()),
    }
}
async fn next(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    request: &ListRequest,
    more: bool,
    last: Option<&str>,
) -> Result<Option<CursorToken>> {
    if more {
        Ok(Some(
            paging::save(
                db,
                actor,
                scope,
                json(&request.limit)?,
                json(&last.ok_or(Error(ErrorCode::InternalError))?)?,
            )
            .await?,
        ))
    } else {
        Ok(None)
    }
}
impl Writer {
    pub async fn register_archive_copy(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &RegisterArchiveCopyRequest,
    ) -> Result<ArchivePhysicalCopy> {
        location(&request.location)?;
        let mut tx = self.transaction().await?;
        collection_access(&mut tx, actor, collection, true).await?;
        let set = disc_access(&mut tx, actor, collection, disc).await?;
        require(
            set.state == ArchiveJobState::Ready,
            ErrorCode::InvalidRequest,
        )?;
        let scope = format!("archive-copy:{}", disc.as_str());
        let key = request.request_id.as_str().parse()?;
        if let Some(value) = replay(&mut tx, actor, &scope, &key, &json(request)?).await? {
            return Ok(value);
        }
        let copy: ArchiveCopyId = id().parse()?;
        sqlx::query("INSERT INTO archive_copies(id,disc_id,label,location,revision,reported_burned_at) VALUES($1,$2,$3,$4,$5,to_timestamp($6::double precision/1000))").bind(copy.as_str()).bind(disc.as_str()).bind(request.label.as_str()).bind(&request.location).bind(id()).bind(request.reported_burned_at_ms.as_ref().map(|n|n.get() as i64)).execute(&mut *tx).await?;
        let result = load_copy(&mut tx, &copy).await?;
        event(&mut tx, actor, &result, "registered").await?;
        remember(&mut tx, actor, &scope, &key, json(request)?, json(&result)?).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn update_archive_copy(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &UpdateArchiveCopyRequest,
    ) -> Result<ArchivePhysicalCopy> {
        request.validate()?;
        location(&request.location)?;
        let mut tx = self.transaction().await?;
        let old = copy_access(&mut tx, actor, collection, copy, true).await?;
        require(
            old.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        sqlx::query("UPDATE archive_copies SET label=$2,location=$3,health=$4,revision=$5,reported_burned_at=CASE WHEN $6::bigint IS NULL THEN reported_burned_at ELSE to_timestamp($6::double precision/1000) END WHERE id=$1").bind(copy.as_str()).bind(request.label.as_str()).bind(&request.location).bind(json(request.health.as_ref().unwrap_or(&old.health))?.as_str().ok_or(Error(ErrorCode::InvalidRequest))?).bind(id()).bind(request.reported_burned_at_ms.as_ref().map(|n|n.get() as i64)).execute(&mut *tx).await?;
        let result = load_copy(&mut tx, copy).await?;
        event(&mut tx, actor, &result, "updated").await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn list_archive_copies(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &ListRequest,
    ) -> Result<ArchiveCopyPage> {
        let mut tx = self.transaction().await?;
        disc_access(&mut tx, actor, collection, disc).await?;
        let scope = format!("archive-copies:{}", disc.as_str());
        let cursor = after(&mut tx, actor, &scope, request).await?;
        let rows = sqlx::query(&format!(
            "{COPY_SELECT} WHERE c.disc_id=$1 AND c.id>$2 ORDER BY c.id LIMIT $3"
        ))
        .bind(disc.as_str())
        .bind(cursor)
        .bind(request.limit.get() as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
        let more = rows.len() > request.limit.get() as usize;
        let copies: Vec<_> = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(copy_row)
            .collect::<Result<_>>()?;
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            copies.last().map(|c| c.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveCopyPage {
            copies,
            next_cursor,
        })
    }
}

const REPORT_SELECT: &str = "SELECT r.*,(extract(epoch FROM r.received_at)*1000)::bigint AS received_ms FROM archive_reports r";
fn report_row(row: &PgRow) -> Result<ArchiveVerificationReport> {
    Ok(ArchiveVerificationReport {
        id: row.get::<String, _>("id").parse()?,
        principal_id: row.get::<String, _>("principal_id").parse()?,
        copy_id: row.get::<String, _>("copy_id").parse()?,
        received_at_ms: (row.get::<i64, _>("received_ms") as u64).try_into()?,
        observation: serde_json::from_value(row.get("observation"))?,
        complete_read_back: row.get("complete_read_back"),
    })
}
async fn refresh_completion(db: &mut PgConnection, collection: &ArchiveCollectionId) -> Result<()> {
    let after: Option<i64> = sqlx::query_scalar(
        "SELECT after_sequence FROM archive_verification_queue WHERE collection_id=$1 FOR UPDATE",
    )
    .bind(collection.as_str())
    .fetch_optional(&mut *db)
    .await?;
    let Some(after) = after else {
        return Ok(());
    };
    let sets = sqlx::query("SELECT id,sequence FROM archive_sets WHERE collection_id=$1 AND state='ready' AND verified_complete_at IS NULL AND sequence>$2 ORDER BY sequence LIMIT 32").bind(collection.as_str()).bind(after).fetch_all(&mut *db).await?;
    for set in &sets {
        sqlx::query("UPDATE archive_sets SET verified_complete_at=clock_timestamp(),revision=$2 WHERE id=$1 AND verified_complete_at IS NULL AND NOT EXISTS(SELECT 1 FROM (SELECT id AS disc FROM archive_discs WHERE set_id=$1 UNION SELECT disc_id FROM archive_placements WHERE set_id=$1) required WHERE NOT EXISTS(SELECT 1 FROM archive_copies c WHERE c.disc_id=required.disc AND c.health='read_back_verified'))").bind(set.get::<String,_>("id")).bind(id()).execute(&mut *db).await?;
    }
    if sets.len() < 32 {
        sqlx::query("DELETE FROM archive_verification_queue WHERE collection_id=$1")
            .bind(collection.as_str())
            .execute(db)
            .await?;
    } else {
        sqlx::query(
            "UPDATE archive_verification_queue SET after_sequence=$2,next_attempt_at=clock_timestamp() WHERE collection_id=$1",
        )
        .bind(collection.as_str())
        .bind(
            sets.last()
                .ok_or(Error(ErrorCode::InternalError))?
                .get::<i64, _>("sequence"),
        )
        .execute(db)
        .await?;
    }
    Ok(())
}
impl Writer {
    pub async fn submit_archive_verification(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &SubmitArchiveVerificationRequest,
    ) -> Result<ArchiveVerificationReport> {
        let mut tx = self.transaction().await?;
        let old = copy_access(&mut tx, actor, collection, copy, true).await?;
        let scope = format!("archive-report:{}", copy.as_str());
        let key = request.request_id.as_str().parse()?;
        if let Some(value) = replay(&mut tx, actor, &scope, &key, &json(request)?).await? {
            return Ok(value);
        }
        require(
            old.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        require(
            request.collection_id == *collection && old.disc_id == request.disc_id,
            ErrorCode::InvalidRequest,
        )?;
        let disc = sqlx::query("SELECT * FROM archive_discs WHERE id=$1")
            .bind(old.disc_id.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let expected = disc.get::<i64, _>("object_count") as u64;
        let bytes = disc.get::<i64, _>("payload_bytes") as u64;
        require(
            request.index_sha256.as_str() == disc.get::<String, _>("index_sha256")
                && request.bundle_sha256.as_str() == disc.get::<String, _>("bundle_sha256")
                && request.payload_inventory_sha256.as_str()
                    == disc.get::<String, _>("inventory_sha256")
                && request.expected_objects.get() == expected,
            ErrorCode::IntegrityFailure,
        )?;
        require(
            request.verified_objects.get() <= expected
                && request.verified_bytes.get() <= bytes
                && (request.verified_objects.get() != 0 || request.verified_bytes.get() == 0),
            ErrorCode::InvalidRequest,
        )?;
        let complete = request.metadata_verified
            && request.verified_objects.get() == expected
            && request.verified_bytes.get() == bytes;
        let report = id();
        let row = sqlx::query("INSERT INTO archive_reports(id,copy_id,principal_id,observation,complete_read_back) VALUES($1,$2,$3,$4,$5) RETURNING *, (extract(epoch FROM received_at)*1000)::bigint AS received_ms").bind(&report).bind(copy.as_str()).bind(actor.principal_id()).bind(json(request)?).bind(complete).fetch_one(&mut *tx).await?;
        sqlx::query(
            "UPDATE archive_copies SET revision=$2,latest_report_id=$3,health=$4 WHERE id=$1",
        )
        .bind(copy.as_str())
        .bind(id())
        .bind(&report)
        .bind(if complete {
            "read_back_verified"
        } else {
            "damaged"
        })
        .execute(&mut *tx)
        .await?;
        let current = load_copy(&mut tx, copy).await?;
        event(&mut tx, actor, &current, "read_back").await?;
        sqlx::query("INSERT INTO archive_verification_queue(collection_id) VALUES($1) ON CONFLICT(collection_id) DO UPDATE SET after_sequence=0").bind(collection.as_str()).execute(&mut *tx).await?;
        refresh_completion(&mut tx, collection).await?;
        let result = report_row(&row)?;
        remember(&mut tx, actor, &scope, &key, json(request)?, json(&result)?).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn list_archive_reports(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &ListRequest,
    ) -> Result<ArchiveReportPage> {
        let mut tx = self.transaction().await?;
        copy_access(&mut tx, actor, collection, copy, false).await?;
        let scope = format!("archive-reports:{}", copy.as_str());
        let cursor = after(&mut tx, actor, &scope, request).await?;
        let rows = sqlx::query(&format!(
            "{REPORT_SELECT} WHERE r.copy_id=$1 AND r.id>$2 ORDER BY r.id LIMIT $3"
        ))
        .bind(copy.as_str())
        .bind(cursor)
        .bind(request.limit.get() as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
        let more = rows.len() > request.limit.get() as usize;
        let reports: Vec<_> = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(report_row)
            .collect::<Result<_>>()?;
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            reports.last().map(|r| r.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveReportPage {
            reports,
            next_cursor,
        })
    }
    /// Finish bounded historical-completion updates after large report imports.
    pub async fn archive_verification_tick(&self) -> Result<()> {
        let mut tx = self.transaction().await?;
        let collections: Vec<String> = sqlx::query_scalar(
            "SELECT collection_id FROM archive_verification_queue ORDER BY next_attempt_at,collection_id LIMIT 4",
        )
        .fetch_all(&mut *tx)
        .await?;
        drop(tx);
        for collection in collections {
            let mut tx = self.transaction().await?;
            // The same collection lock serializes reports, damage and completion.
            sqlx::query("SELECT id FROM archive_collections WHERE id=$1 FOR UPDATE")
                .bind(&collection)
                .fetch_one(&mut *tx)
                .await?;
            refresh_completion(&mut tx, &collection.parse()?).await?;
            tx.commit().await?;
        }
        Ok(())
    }
}

impl Writer {
    pub async fn list_archive_copy_events(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        copy: &ArchiveCopyId,
        request: &ListRequest,
    ) -> Result<ArchiveCopyEventPage> {
        let mut tx = self.transaction().await?;
        copy_access(&mut tx, actor, collection, copy, false).await?;
        let scope = format!("archive-copy-events:{}", copy.as_str());
        let cursor = after(&mut tx, actor, &scope, request).await?;
        let rows = sqlx::query("SELECT *, (extract(epoch FROM received_at)*1000)::bigint AS received_ms FROM archive_copy_events WHERE copy_id=$1 AND id>$2 ORDER BY id LIMIT $3").bind(copy.as_str()).bind(cursor).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let mut events = Vec::new();
        for row in rows.iter().take(request.limit.get() as usize) {
            events.push(ArchiveCopyEvent {
                id: row.get("id"),
                copy_id: copy.clone(),
                principal_id: row.get::<String, _>("principal_id").parse()?,
                received_at_ms: (row.get::<i64, _>("received_ms") as u64).try_into()?,
                kind: serde_json::from_value(json(&row.get::<String, _>("kind"))?)?,
                observation: serde_json::from_value(row.get("observation"))?,
            });
        }
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            events.last().map(|e| e.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveCopyEventPage {
            events,
            next_cursor,
        })
    }
}
