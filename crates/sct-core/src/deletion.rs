//! Ordinary deletion changes visibility only; purge is a separate operation.
use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    lease::Writer,
    roots::{json, lock_root, remember, replay, revision},
};
use sct_protocol::*;
use simple_server::database::postgres as db;

impl Writer {
    pub async fn hide_entry(
        &self,
        actor: &Actor,
        root: &RootId,
        entry: &EntryId,
        request: &HideEntryRequest,
    ) -> Result<EntryInspection> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Delete).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Delete).await?;
        let scope = format!("hide:{}:{}", root.as_str(), entry.as_str());
        if let Some(result) = replay(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            &json(request)?,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(result);
        }
        lock_root(&mut tx, root.as_str()).await?;
        let enabled: bool = db::query_scalar("SELECT state='enabled' FROM roots WHERE id=$1")
            .bind(root.as_str())
            .fetch_one(&mut *tx)
            .await?;
        require(enabled, ErrorCode::DestinationUnavailable)?;
        let current = db::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2")
            .bind(root.as_str())
            .bind(entry.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        revision(&current, &request.expected_revision)?;
        let parent: String = current
            .get::<Option<String>, _>("parent_id")
            .ok_or(Error(ErrorCode::InvalidRequest))?;
        require(
            !crate::namespace::hidden(&mut tx, entry.as_str()).await?,
            ErrorCode::NotFound,
        )?;
        // Atomic children have no mutable namespace row and cannot reach here.
        let changed = db::query(
            "UPDATE entries SET hidden=true,revision=$3 WHERE root_id=$1 AND id=$2 RETURNING *",
        )
        .bind(root.as_str())
        .bind(entry.as_str())
        .bind(id())
        .fetch_one(&mut *tx)
        .await?;
        db::query("UPDATE entries SET revision=$2 WHERE id=$1")
            .bind(parent)
            .bind(id())
            .execute(&mut *tx)
            .await?;
        db::query("UPDATE roots SET revision=$2 WHERE id=$1")
            .bind(root.as_str())
            .bind(id())
            .execute(&mut *tx)
            .await?;
        let result = crate::namespace::inspect(&changed)?;
        remember(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            json(request)?,
            json(&result)?,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
}
