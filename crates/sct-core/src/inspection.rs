//! Read-only root-scoped operational views. Pages never authorize content changes.
use crate::{
    auth::{self, Actor},
    error::Result,
    lease::Writer,
    paging,
    roots::{json, parse},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

async fn scope(db: &mut PgConnection, actor: &Actor, root: Option<&RootId>) -> Result<()> {
    match root {
        Some(root) => auth::authorize(db, actor, root.as_str(), Capability::ListRead).await,
        None => auth::admin(db, actor).await,
    }
}
async fn after(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    request: &ListRequest,
) -> Result<String> {
    match &request.cursor {
        Some(token) => paging::consume(db, actor, scope, token, &json(&request.limit)?).await,
        None => Ok(String::new()),
    }
}
async fn next(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    request: &ListRequest,
    rows: &[PgRow],
) -> Result<Option<CursorToken>> {
    if rows.len() > request.limit.get() as usize {
        Ok(Some(
            paging::save(
                db,
                actor,
                scope,
                json(&request.limit)?,
                json(&rows[request.limit.get() as usize - 1].get::<String, _>("id"))?,
            )
            .await?,
        ))
    } else {
        Ok(None)
    }
}
fn state<T: serde::de::DeserializeOwned>(r: &PgRow, column: &str) -> Result<T> {
    Ok(serde_json::from_value(json(&r.get::<String, _>(column))?)?)
}
fn failure(r: &PgRow) -> Result<Option<ErrorCode>> {
    r.get::<Option<String>, _>("failure")
        .map(|s| Ok(serde_json::from_value(json(&s)?)?))
        .transpose()
}
impl Writer {
    pub async fn root_usage(&self, actor: &Actor, root: &RootId) -> Result<RootCapacityUsage> {
        let mut tx = self.transaction().await?;
        scope(&mut tx, actor, Some(root)).await?;
        // One SQL snapshot, graph set-union in PostgreSQL, aggregate-only results.
        let r=sqlx::query("WITH RECURSIVE retained(digest) AS (SELECT manifest FROM file_versions WHERE root_id=$1 AND NOT retired UNION SELECT e.child FROM retained r JOIN content_edges e ON e.root_id=$1 AND e.parent=r.digest) SELECT (SELECT COALESCE(sum(length),0)::bigint FROM file_versions WHERE root_id=$1 AND NOT retired) AS logical, (SELECT COALESCE(sum(o.length),0)::bigint FROM retained r JOIN content_objects o ON o.root_id=$1 AND o.digest=r.digest WHERE o.kind='chunk') AS unique_bytes, (SELECT COALESCE(sum(c.bytes),0)::bigint FROM object_copies c WHERE c.root_id=$1 AND c.state<>'deleted') AS physical, (SELECT COALESCE(sum(c.bytes),0)::bigint FROM object_copies c JOIN allocations a ON a.id=c.allocation_id WHERE c.root_id=$1 AND c.state<>'deleted' AND a.root_id IS NULL) AS staging, (SELECT COALESCE(sum(r.bytes),0)::bigint FROM reservations r JOIN operations o ON o.id=r.operation_id WHERE o.root_id=$1) AS reserved, (SELECT COALESCE(sum(c.bytes),0)::bigint FROM object_copies c WHERE c.root_id=$1 AND c.state<>'deleted' AND c.pending_charge) AS pending").bind(root.as_str()).fetch_one(&mut *tx).await?;
        Ok(RootCapacityUsage {
            root_id: root.clone(),
            logical_retained_bytes: (r.get::<i64, _>("logical") as u64).try_into()?,
            unique_retained_bytes: (r.get::<i64, _>("unique_bytes") as u64).try_into()?,
            physical_occupied_bytes: (r.get::<i64, _>("physical") as u64).try_into()?,
            staging_occupied_bytes: (r.get::<i64, _>("staging") as u64).try_into()?,
            reserved_bytes: (r.get::<i64, _>("reserved") as u64).try_into()?,
            pending_cleanup_bytes: (r.get::<i64, _>("pending") as u64).try_into()?,
        })
    }
    pub async fn list_transfer_activity(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<TransferActivityPage> {
        let mut tx = self.transaction().await?;
        scope(&mut tx, actor, Some(root)).await?;
        let name = format!("activity:{}", root.as_str());
        let after = after(&mut tx, actor, &name, request).await?;
        let rows=sqlx::query("WITH sessions AS (SELECT s.id,s.root_id,s.principal_id,CASE WHEN d.id IS NULL THEN 'file' ELSE 'directory' END AS kind,s.state,o.phase,s.admission,s.expires_at,s.failure FROM transfer_sessions s JOIN operations o ON o.id=s.operation_id LEFT JOIN tree_drafts d ON d.id=s.id WHERE s.root_id=$1 UNION ALL SELECT d.id,d.root_id,d.principal_id,'directory',d.state,d.phase,NULL,d.expires_at,NULL FROM tree_drafts d WHERE d.root_id=$1 AND NOT EXISTS(SELECT 1 FROM transfer_sessions s WHERE s.id=d.id)) SELECT *, (extract(epoch FROM expires_at)*1000)::bigint AS expires_ms FROM sessions WHERE id>$2 ORDER BY id LIMIT $3").bind(root.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = next(&mut tx, actor, &name, request, &rows).await?;
        let sessions = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(TransferActivity {
                    id: r.get::<String, _>("id").parse()?,
                    root_id: root.clone(),
                    principal_id: r.get::<String, _>("principal_id").parse()?,
                    kind: r.get("kind"),
                    state: state(r, "state")?,
                    phase: r.get("phase"),
                    admission: r
                        .get::<Option<String>, _>("admission")
                        .map(|s| serde_json::from_value(serde_json::json!(s)))
                        .transpose()?,
                    expires_at_ms: (r.get::<i64, _>("expires_ms") as u64).try_into()?,
                    failure: failure(r)?,
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(TransferActivityPage {
            sessions,
            next_cursor,
        })
    }
    pub async fn list_job_activity(
        &self,
        actor: &Actor,
        root: Option<&RootId>,
        request: &ListRequest,
    ) -> Result<JobActivityPage> {
        let mut tx = self.transaction().await?;
        scope(&mut tx, actor, root).await?;
        let name = format!("jobs:{}", root.map_or("*", RootId::as_str));
        let after = after(&mut tx, actor, &name, request).await?;
        let rows=sqlx::query("SELECT o.*,j.operation_id AS lifecycle_id,j.processed,j.total,j.failure,j.state AS lifecycle_state,j.phase AS lifecycle_phase,(SELECT id FROM transfer_sessions WHERE operation_id=o.id) AS upload_id FROM operations o LEFT JOIN lifecycle_jobs j ON j.operation_id=o.id WHERE ($1::text IS NULL OR o.root_id=$1) AND o.id>$2 ORDER BY o.id LIMIT $3").bind(root.map(RootId::as_str)).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = next(&mut tx, actor, &name, request, &rows).await?;
        let mut jobs = Vec::new();
        for r in rows.iter().take(request.limit.get() as usize) {
            let root_id: RootId = r.get::<String, _>("root_id").parse()?;
            let lifecycle = r
                .get::<Option<String>, _>("lifecycle_state")
                .map(|v| -> Result<LifecycleJob> {
                    Ok(LifecycleJob {
                        id: r.get::<String, _>("id").parse()?,
                        root_id: root_id.clone(),
                        state: serde_json::from_value(serde_json::json!(v))?,
                        phase: r.get("lifecycle_phase"),
                        processed: (r.get::<Option<i64>, _>("processed").unwrap_or(0) as u64)
                            .try_into()?,
                        total: r
                            .get::<Option<i64>, _>("total")
                            .map(|n| (n as u64).try_into())
                            .transpose()?,
                        failure: failure(r)?,
                    })
                })
                .transpose()?;
            jobs.push(JobActivity {
                phase: r.get("phase"),
                lifecycle,
                operation: OperationInspection {
                    id: r.get::<String, _>("id").parse()?,
                    root_id,
                    owner_principal_id: r
                        .get::<Option<String>, _>("principal_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    kind: state(r, "kind")?,
                    state: state(r, "state")?,
                    upload_id: r
                        .get::<Option<String>, _>("upload_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    completed_units: (r.get::<Option<i64>, _>("processed").unwrap_or(0) as u64)
                        .try_into()?,
                    total_units: r
                        .get::<Option<i64>, _>("total")
                        .map(|n| (n as u64).try_into())
                        .transpose()?,
                    retry: RetryGuidance::Never,
                    failure_code: failure(r)?,
                },
            });
        }
        tx.commit().await?;
        Ok(JobActivityPage { jobs, next_cursor })
    }
    pub async fn list_audit(
        &self,
        actor: &Actor,
        root: Option<&RootId>,
        request: &ListRequest,
    ) -> Result<AuditPage> {
        let mut tx = self.transaction().await?;
        scope(&mut tx, actor, root).await?;
        let name = format!("audit:{}", root.map_or("*", RootId::as_str));
        let before: Option<(i64, String, i64)> = match &request.cursor {
            Some(token) => {
                Some(paging::consume(&mut tx, actor, &name, token, &json(&request.limit)?).await?)
            }
            None => None,
        };
        let rows=sqlx::query("WITH events AS (SELECT 'management' AS source,sequence,root_id,principal_id,event,details,(extract(epoch FROM created_at)*1000000)::bigint AS created_us FROM management_audit WHERE $1::text IS NULL OR root_id=$1 UNION ALL SELECT 'transfer',a.sequence,a.root_id,o.principal_id,a.event,a.details,(extract(epoch FROM a.created_at)*1000000)::bigint FROM transfer_audit a JOIN operations o ON o.id=a.operation_id WHERE $1::text IS NULL OR a.root_id=$1) SELECT * FROM events WHERE $2::bigint IS NULL OR (created_us,source,sequence)<($2,$3,$4) ORDER BY created_us DESC,source DESC,sequence DESC LIMIT $5").bind(root.map(RootId::as_str)).bind(before.as_ref().map(|b|b.0)).bind(before.as_ref().map(|b|&b.1)).bind(before.as_ref().map(|b|b.2)).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = if rows.len() > request.limit.get() as usize {
            let r = &rows[request.limit.get() as usize - 1];
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    &name,
                    json(&request.limit)?,
                    json(&(
                        r.get::<i64, _>("created_us"),
                        r.get::<String, _>("source"),
                        r.get::<i64, _>("sequence"),
                    ))?,
                )
                .await?,
            )
        } else {
            None
        };
        let events = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(AuditEvent {
                    id: format!(
                        "{}-{}",
                        r.get::<String, _>("source"),
                        r.get::<i64, _>("sequence")
                    ),
                    root_id: r
                        .get::<Option<String>, _>("root_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    principal_id: r
                        .get::<Option<String>, _>("principal_id")
                        .map(|s| s.parse())
                        .transpose()?,
                    event: r.get("event"),
                    details: parse(r, "details")?,
                    created_at_ms: ((r.get::<i64, _>("created_us") / 1000) as u64).try_into()?,
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(AuditPage {
            events,
            next_cursor,
        })
    }
}
