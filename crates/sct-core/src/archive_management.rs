//! Source authority is independent of optical collection ownership.
use crate::{
    archive_catalog::*,
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    paging,
    roots::{json, remember, replay},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row};

pub(crate) async fn source_access(
    db: &mut PgConnection,
    actor: &Actor,
    selections: &[ArchiveSelection],
) -> Result<()> {
    let roots: std::collections::BTreeSet<_> =
        selections.iter().map(|s| s.root_id.as_str()).collect();
    let admin = auth::administrator(db, actor).await?;
    let roots: Vec<_> = roots.into_iter().collect();
    let rows = sqlx::query("SELECT id,owner_id FROM roots WHERE id=ANY($1) ORDER BY id FOR SHARE")
        .bind(&roots)
        .fetch_all(&mut *db)
        .await?;
    require(rows.len() == roots.len(), ErrorCode::NotFound)?;
    // Read grants in a fresh statement after root locks. A joined lookup could
    // retain a pre-revocation MVCC snapshot while waiting for a revoker's lock.
    let grants = sqlx::query(
        "SELECT root_id,capabilities FROM grants WHERE root_id=ANY($1) AND principal_id=$2",
    )
    .bind(&roots)
    .bind(actor.principal_id())
    .fetch_all(db)
    .await?;
    let grants: std::collections::BTreeMap<String, serde_json::Value> = grants
        .into_iter()
        .map(|r| (r.get("root_id"), r.get("capabilities")))
        .collect();
    for row in rows {
        if admin || row.get::<String, _>("owner_id") == actor.principal_id() {
            continue;
        }
        let value = grants
            .get(&row.get::<String, _>("id"))
            .ok_or(Error(ErrorCode::NotFound))?;
        let capabilities: Vec<Capability> = serde_json::from_value(value.clone())?;
        require(
            capabilities.contains(&Capability::ListRead),
            ErrorCode::Forbidden,
        )?;
    }
    Ok(())
}
pub(crate) async fn set_access(
    db: &mut PgConnection,
    actor: &Actor,
    collection: &ArchiveCollectionId,
    set: &ArchiveSetId,
) -> Result<ArchiveSet> {
    collection_access(db, actor, collection, false).await?;
    let value = load_set(db, collection, set).await?;
    source_access(db, actor, &value.selections).await?;
    Ok(value)
}
pub(crate) async fn disc_access(
    db: &mut PgConnection,
    actor: &Actor,
    collection: &ArchiveCollectionId,
    disc: &ArchiveDiscId,
) -> Result<ArchiveSet> {
    collection_access(db, actor, collection, false).await?;
    let set: String =
        sqlx::query_scalar("SELECT set_id FROM archive_discs WHERE collection_id=$1 AND id=$2")
            .bind(collection.as_str())
            .bind(disc.as_str())
            .fetch_optional(&mut *db)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
    set_access(db, actor, collection, &set.parse()?).await
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
    position: Option<&str>,
) -> Result<Option<CursorToken>> {
    if more {
        Ok(Some(
            paging::save(
                db,
                actor,
                scope,
                json(&request.limit)?,
                json(&position.ok_or(Error(ErrorCode::InternalError))?)?,
            )
            .await?,
        ))
    } else {
        Ok(None)
    }
}
impl Writer {
    pub async fn inspect_archive_collection(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
    ) -> Result<ArchiveCollection> {
        let mut tx = self.transaction().await?;
        collection_access(&mut tx, actor, collection, false).await
    }
    pub async fn update_archive_collection(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        request: &UpdateArchiveCollectionRequest,
    ) -> Result<ArchiveCollection> {
        require(
            request.media_capacity_bytes.get() > 0,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        let old = collection_access(&mut tx, actor, collection, true).await?;
        require(
            old.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        sqlx::query(
            "UPDATE archive_collections SET label=$2,media_capacity=$3,revision=$4 WHERE id=$1",
        )
        .bind(collection.as_str())
        .bind(request.label.as_str())
        .bind(request.media_capacity_bytes.get() as i64)
        .bind(id())
        .execute(&mut *tx)
        .await?;
        let result = collection_access(&mut tx, actor, collection, false).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "archive.collection.updated",
            serde_json::json!({"collection_id":collection}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn list_archive_collections(
        &self,
        actor: &Actor,
        request: &ListRequest,
    ) -> Result<ArchiveCollectionPage> {
        let mut tx = self.transaction().await?;
        let admin = auth::administrator(&mut tx, actor).await?;
        let scope = format!("archive-collections:{admin}");
        let after = after(&mut tx, actor, &scope, request).await?;
        let rows = sqlx::query(&format!(
            "{COLLECTION_SELECT} WHERE c.id>$1 AND ($2 OR c.owner_id=$3) ORDER BY c.id LIMIT $4"
        ))
        .bind(after)
        .bind(admin)
        .bind(actor.principal_id())
        .bind(request.limit.get() as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
        let more = rows.len() > request.limit.get() as usize;
        let collections = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(collection_row)
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            collections.last().map(|c| c.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveCollectionPage {
            collections,
            next_cursor,
        })
    }
    pub async fn inspect_archive_set(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
    ) -> Result<ArchiveSet> {
        let mut tx = self.transaction().await?;
        set_access(&mut tx, actor, collection, set).await
    }
    pub async fn list_archive_sets(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        request: &ListRequest,
    ) -> Result<ArchiveSetPage> {
        let mut tx = self.transaction().await?;
        collection_access(&mut tx, actor, collection, false).await?;
        let scope = format!("archive-sets:{}", collection.as_str());
        let after = after(&mut tx, actor, &scope, request).await?;
        let rows = sqlx::query(&format!(
            "{SET_SELECT} WHERE collection_id=$1 AND id>$2 ORDER BY id LIMIT $3"
        ))
        .bind(collection.as_str())
        .bind(after)
        .bind(request.limit.get() as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
        let more = rows.len() > request.limit.get() as usize;
        let mut sets = Vec::new();
        for row in rows.iter().take(request.limit.get() as usize) {
            let set = set_row(row)?;
            source_access(&mut tx, actor, &set.selections).await?;
            sets.push(set);
        }
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            sets.last().map(|s| s.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveSetPage { sets, next_cursor })
    }
    pub async fn archive_usage(&self, actor: &Actor) -> Result<ArchiveUsage> {
        let mut tx = self.transaction().await?;
        auth::administrator(&mut tx, actor).await?;
        let row=sqlx::query("SELECT quota_bytes,cache_bytes,reserved_bytes FROM archive_accounts WHERE principal_id=$1").bind(actor.principal_id()).fetch_optional(&mut *tx).await?;
        let active:i64=sqlx::query_scalar("SELECT count(*) FROM archive_sets s JOIN archive_collections c ON c.id=s.collection_id WHERE c.owner_id=$1 AND s.state IN ('capturing','planning','awaiting_confirmation','building')").bind(actor.principal_id()).fetch_one(&mut *tx).await?;
        Ok(ArchiveUsage {
            quota_bytes: row
                .as_ref()
                .map_or(107374182400, |r| r.get::<i64, _>("quota_bytes") as u64)
                .try_into()?,
            cache_bytes: row
                .as_ref()
                .map_or(0, |r| r.get::<i64, _>("cache_bytes") as u64)
                .try_into()?,
            reserved_bytes: row
                .as_ref()
                .map_or(0, |r| r.get::<i64, _>("reserved_bytes") as u64)
                .try_into()?,
            active_sets: (active as u64).try_into()?,
        })
    }
    pub async fn confirm_archive_set(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ArchiveSetActionRequest,
    ) -> Result<ArchiveSet> {
        self.archive_set_action(actor, collection, set, request, true)
            .await
    }
    pub async fn cancel_archive_set(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ArchiveSetActionRequest,
    ) -> Result<ArchiveSet> {
        self.archive_set_action(actor, collection, set, request, false)
            .await
    }
    async fn archive_set_action(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ArchiveSetActionRequest,
        confirm: bool,
    ) -> Result<ArchiveSet> {
        let mut tx = self.transaction().await?;
        let col = collection_access(&mut tx, actor, collection, true).await?;
        let old = set_access(&mut tx, actor, collection, set).await?;
        let scope = format!("archive-action:{}:{confirm}", set.as_str());
        let key = request.request_id.as_str().parse()?;
        if let Some(value) = replay(&mut tx, actor, &scope, &key, &json(request)?).await? {
            return Ok(value);
        }
        require(
            old.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        require(!old.state.terminal(), ErrorCode::RevisionConflict)?;
        if confirm {
            require(
                !col.reconciliation_required,
                ErrorCode::ArchiveReconciliationRequired,
            )?;
            require(
                old.state == ArchiveJobState::AwaitingConfirmation && old.plan.is_some(),
                ErrorCode::RevisionConflict,
            )?;
            crate::archive_planning::validate_reuse(&mut tx, actor, set).await?;
            let updated=sqlx::query("UPDATE archive_sets SET state='building',frozen_at=clock_timestamp(),revision=$2 WHERE id=$1 AND expires_at>clock_timestamp()").bind(set.as_str()).bind(id()).execute(&mut *tx).await?.rows_affected();
            require(updated == 1, ErrorCode::ArchiveExpired)?;
        } else {
            sqlx::query("UPDATE archive_sets SET state='cancelled',revision=$2 WHERE id=$1")
                .bind(set.as_str())
                .bind(id())
                .execute(&mut *tx)
                .await?;
        }
        let result = load_set(&mut tx, collection, set).await?;
        remember(&mut tx, actor, &scope, &key, json(request)?, json(&result)?).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            if confirm {
                "archive.plan.confirmed"
            } else {
                "archive.cancelled"
            },
            serde_json::json!({"collection_id":collection,"set_id":set}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Each asynchronous batch must call this with its captured credential before
    /// reading more source data. Offline media already obtained cannot be revoked.
    pub async fn authorize_archive_work(&self, set: &ArchiveSetId) -> Result<bool> {
        let mut tx = self.transaction().await?;
        let row = sqlx::query(
            "SELECT collection_id,principal_id,credential_id,state FROM archive_sets WHERE id=$1",
        )
        .bind(set.as_str())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
        let state: ArchiveJobState = serde_json::from_value(json(&row.get::<String, _>("state"))?)?;
        if state.terminal() {
            return Ok(false);
        }
        let actor = Actor {
            principal: row.get("principal_id"),
            credential: row.get("credential_id"),
        };
        let collection = row.get::<String, _>("collection_id").parse()?;
        match set_access(&mut tx, &actor, &collection, set).await {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.0,
                    ErrorCode::Unauthorized | ErrorCode::Forbidden | ErrorCode::NotFound
                ) =>
            {
                sqlx::query("UPDATE archive_sets SET state='revoked',failure='forbidden',revision=$2 WHERE id=$1 AND state IN ('capturing','planning','awaiting_confirmation','building')").bind(set.as_str()).bind(id()).execute(&mut *tx).await?;
                crate::management::audit(
                    &mut tx,
                    &actor,
                    None,
                    "archive.authority.revoked",
                    serde_json::json!({"collection_id":collection,"set_id":set}),
                )
                .await?;
                tx.commit().await?;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    /// Downloads and read-back surfaces must authorize the original disc's whole
    /// index, including sources that a later snapshot may no longer reference.
    pub async fn authorize_archive_disc(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        disc_access(&mut tx, actor, collection, disc).await?;
        Ok(())
    }
}
impl Writer {
    pub async fn list_archive_discs(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ListRequest,
    ) -> Result<ArchiveDiscPage> {
        let mut tx = self.transaction().await?;
        let snapshot = set_access(&mut tx, actor, collection, set).await?;
        let scope = format!(
            "archive-discs:{}:{}",
            set.as_str(),
            snapshot.revision.as_str()
        );
        let after = after(&mut tx, actor, &scope, request).await?;
        let rows=sqlx::query("SELECT d.*,a.id AS artifact_id,CASE WHEN a.state='available' AND (a.cleanup_pending OR a.expires_at<=clock_timestamp()) THEN 'expired' ELSE a.state END AS artifact_state,a.sha256 AS artifact_sha256,a.length AS artifact_length,(extract(epoch FROM a.expires_at)*1000)::bigint AS artifact_expires,a.download_count FROM archive_discs d LEFT JOIN archive_artifacts a ON a.disc_id=d.id WHERE d.collection_id=$1 AND d.id>$3 AND (d.set_id=$2 OR EXISTS(SELECT 1 FROM archive_placements p WHERE p.set_id=$2 AND p.disc_id=d.id)) ORDER BY d.id LIMIT $4")
            .bind(collection.as_str()).bind(set.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let mut discs = Vec::new();
        for row in rows.iter().take(request.limit.get() as usize) {
            let identity: ArchiveDiscId = row.get::<String, _>("id").parse()?;
            disc_access(&mut tx, actor, collection, &identity).await?;
            let artifact = if let Some(identity) = row.get::<Option<String>, _>("artifact_id") {
                Some(ArchiveArtifact {
                    id: identity.parse()?,
                    state: serde_json::from_value(json(&row.get::<String, _>("artifact_state"))?)?,
                    sha256: row
                        .get::<Option<String>, _>("artifact_sha256")
                        .map(|s| s.parse())
                        .transpose()?,
                    length: row
                        .get::<Option<i64>, _>("artifact_length")
                        .map(|n| (n as u64).try_into())
                        .transpose()?,
                    expires_at_ms: (row.get::<i64, _>("artifact_expires") as u64).try_into()?,
                    download_count: (row.get::<i64, _>("download_count") as u64).try_into()?,
                })
            } else {
                None
            };
            discs.push(ArchiveLogicalDisc {
                id: identity,
                collection_id: collection.clone(),
                set_id: row.get::<String, _>("set_id").parse()?,
                number: (row.get::<i64, _>("number") as u64).try_into()?,
                index_sha256: row.get::<String, _>("index_sha256").parse()?,
                bundle_sha256: row.get::<String, _>("bundle_sha256").parse()?,
                payload_bytes: (row.get::<i64, _>("payload_bytes") as u64).try_into()?,
                object_count: (row.get::<i64, _>("object_count") as u64).try_into()?,
                payload_inventory_sha256: row.get::<String, _>("inventory_sha256").parse()?,
                planned_iso_bytes: (row.get::<i64, _>("planned_iso_bytes") as u64).try_into()?,
                artifact,
            });
        }
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            discs.last().map(|d| d.id.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveDiscPage { discs, next_cursor })
    }
    pub async fn list_archive_entries(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        set: &ArchiveSetId,
        request: &ListRequest,
    ) -> Result<ArchiveEntryPage> {
        let mut tx = self.transaction().await?;
        let snapshot = set_access(&mut tx, actor, collection, set).await?;
        let scope = format!(
            "archive-entries:{}:{}",
            set.as_str(),
            snapshot.revision.as_str()
        );
        let after = after(&mut tx, actor, &scope, request).await?;
        let rows=sqlx::query("SELECT path,kind,root_id,version_id,length FROM archive_entries WHERE set_id=$1 AND path>$2 ORDER BY path LIMIT $3").bind(set.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let entries = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(ArchiveEntry {
                    path: r.get::<String, _>("path").parse()?,
                    kind: serde_json::from_value(json(&r.get::<String, _>("kind"))?)?,
                    root_id: r
                        .get::<Option<String>, _>("root_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    version_id: r
                        .get::<Option<String>, _>("version_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    length: (r.get::<i64, _>("length") as u64).try_into()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = next(
            &mut tx,
            actor,
            &scope,
            request,
            more,
            entries.last().map(|e| e.path.as_str()),
        )
        .await?;
        tx.commit().await?;
        Ok(ArchiveEntryPage {
            entries,
            next_cursor,
        })
    }
    pub async fn list_archive_objects(
        &self,
        actor: &Actor,
        collection: &ArchiveCollectionId,
        disc: &ArchiveDiscId,
        request: &ListRequest,
    ) -> Result<ArchiveObjectPage> {
        let mut tx = self.transaction().await?;
        let snapshot = disc_access(&mut tx, actor, collection, disc).await?;
        let scope = format!(
            "archive-objects:{}:{}",
            disc.as_str(),
            snapshot.revision.as_str()
        );
        let (root, digest): (String, String) = match &request.cursor {
            Some(token) => {
                paging::consume(&mut tx, actor, &scope, token, &json(&request.limit)?).await?
            }
            None => (String::new(), String::new()),
        };
        let rows=sqlx::query("SELECT root_id,digest,length FROM archive_placements WHERE set_id=$1 AND disc_id=$2 AND (root_id,digest)>($3,$4) ORDER BY root_id,digest LIMIT $5").bind(snapshot.id.as_str()).bind(disc.as_str()).bind(root).bind(digest).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let objects = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(ArchiveDiscObject {
                    root_id: r.get::<String, _>("root_id").parse()?,
                    digest: r.get::<String, _>("digest").parse()?,
                    length: (r.get::<i64, _>("length") as u64).try_into()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            let last = objects.last().ok_or(Error(ErrorCode::InternalError))?;
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    json(&request.limit)?,
                    json(&(last.root_id.as_str(), last.digest.as_str()))?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(ArchiveObjectPage {
            objects,
            next_cursor,
        })
    }
}
