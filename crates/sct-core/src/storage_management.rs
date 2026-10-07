//! Administrator-owned infrastructure, persisted independently of bootstrap TOML.
use crate::{
    auth::{self, Actor},
    backend::Registry,
    error::{Error, Result, id, require},
    lease::Writer,
    management::audit,
    paging,
    roots::{BackendSetup, json, lock_root, parse, revision},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

fn backend(row: &PgRow) -> Result<BackendConfiguration> {
    Ok(BackendConfiguration {
        inspection: parse(row, "inspection")?,
        pool_id: row.get::<String, _>("pool_id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        settings: row.get("settings"),
    })
}
fn allocation(row: &PgRow) -> Result<AllocationManagement> {
    Ok(AllocationManagement {
        revision: row.get::<String, _>("revision").parse()?,
        inspection: AllocationInspection {
            id: row.get::<String, _>("id").parse()?,
            pool_id: row.get::<String, _>("pool_id").parse()?,
            owner: match row.get::<Option<String>, _>("root_id") {
                Some(r) => AllocationOwner::Root {
                    root_id: r.parse()?,
                },
                None => AllocationOwner::ServerStaging,
            },
            capacity_bytes: (row.get::<i64, _>("capacity") as u64).try_into()?,
            reserved_bytes: (row.get::<i64, _>("reserved") as u64).try_into()?,
            occupied_bytes: (row.get::<i64, _>("occupied") as u64).try_into()?,
            pending_cleanup_bytes: (row.get::<i64, _>("pending_cleanup") as u64).try_into()?,
        },
    })
}
fn pool(row: &PgRow) -> Result<PoolInspection> {
    Ok(PoolInspection {
        id: row.get::<String, _>("id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        allocatable_bytes: (row.get::<i64, _>("capacity") as u64).try_into()?,
        allocated_bytes: (row.get::<i64, _>("allocated") as u64).try_into()?,
        reported_available_bytes: None,
    })
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
impl Writer {
    /// Internal runtime startup. Only provisioned references are stored, never resolved secrets.
    pub async fn configured_backends(&self) -> Result<Vec<BackendSetup>> {
        let mut tx = self.transaction().await?;
        let rows=sqlx::query("SELECT b.pool_id,b.inspection,c.settings FROM backends b JOIN backend_configurations c ON c.backend_id=b.id ORDER BY b.id").fetch_all(&mut *tx).await?;
        rows.iter()
            .map(|r| {
                Ok(BackendSetup {
                    pool_id: r.get::<String, _>("pool_id").parse()?,
                    inspection: parse(r, "inspection")?,
                    settings: r.get("settings"),
                })
            })
            .collect()
    }
    pub async fn backend_configuration(
        &self,
        actor: &Actor,
        backend_id: &BackendId,
    ) -> Result<BackendConfiguration> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row=sqlx::query("SELECT b.*,c.revision,c.settings FROM backends b JOIN backend_configurations c ON c.backend_id=b.id WHERE b.id=$1").bind(backend_id.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        backend(&row)
    }
    pub async fn list_backends(&self, actor: &Actor, request: &ListRequest) -> Result<BackendPage> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let after = after(&mut tx, actor, "backends", request).await?;
        let rows=sqlx::query("SELECT b.*,c.revision,c.settings FROM backends b JOIN backend_configurations c ON c.backend_id=b.id WHERE b.id>$1 ORDER BY b.id LIMIT $2").bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = next(&mut tx, actor, "backends", request, &rows).await?;
        let backends = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(backend)
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(BackendPage {
            backends,
            next_cursor,
        })
    }
    pub async fn configure_backend(
        &self,
        actor: &Actor,
        registry: &Registry,
        request: &ConfigureBackendRequest,
    ) -> Result<BackendConfiguration> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        // Serialize instance changes and bootstrap import; destination identity never changes.
        sqlx::query("SELECT pg_advisory_xact_lock(73628492)")
            .execute(&mut *tx)
            .await?;
        let identity = registry.validate(&request.inspection, &request.settings)?;
        let old=sqlx::query("SELECT b.*,c.revision FROM backends b JOIN backend_configurations c ON c.backend_id=b.id WHERE b.id=$1 FOR UPDATE OF b,c").bind(request.inspection.id.as_str()).fetch_optional(&mut *tx).await?;
        match (&old, &request.expected_revision) {
            (Some(row), Some(expected)) => {
                revision(row, expected)?;
                require(
                    row.get::<String, _>("destination_identity") == identity
                        && row.get::<String, _>("kind") == request.inspection.adapter_kind
                        && row.get::<String, _>("pool_id") == request.pool_id.as_str(),
                    ErrorCode::UnsupportedOperation,
                )?;
            }
            (None, None) => {}
            _ => return Err(Error(ErrorCode::RevisionConflict)),
        }
        let mut inspection = request.inspection.clone();
        // Caller hints do not constitute health observations.
        inspection.observed_health = registry
            .get(&inspection.adapter_kind)?
            .probe(&request.settings);
        inspection.last_observed_at_ms = None;
        let rev = id();
        sqlx::query("INSERT INTO backends(id,pool_id,kind,destination_identity,inspection) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET inspection=EXCLUDED.inspection")
            .bind(inspection.id.as_str()).bind(request.pool_id.as_str()).bind(&inspection.adapter_kind).bind(identity).bind(json(&inspection)?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO backend_configurations(backend_id,revision,settings) VALUES($1,$2,$3) ON CONFLICT(backend_id) DO UPDATE SET revision=EXCLUDED.revision,settings=EXCLUDED.settings,updated_at=clock_timestamp()")
            .bind(inspection.id.as_str()).bind(&rev).bind(&request.settings).execute(&mut *tx).await?;
        audit(&mut tx,actor,None,"backend.configured",serde_json::json!({"backend_id":inspection.id,"pool_id":request.pool_id,"adapter_kind":inspection.adapter_kind})).await?;
        tx.commit().await?;
        Ok(BackendConfiguration {
            inspection,
            pool_id: request.pool_id.clone(),
            revision: rev.parse()?,
            settings: request.settings.clone(),
        })
    }
    pub async fn list_pools(&self, actor: &Actor, request: &ListRequest) -> Result<PoolPage> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let after = after(&mut tx, actor, "pools", request).await?;
        let rows=sqlx::query("SELECT p.*,COALESCE((SELECT sum(capacity)::bigint FROM allocations WHERE pool_id=p.id),0) AS allocated FROM pools p WHERE id>$1 ORDER BY id LIMIT $2").bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let next_cursor = next(&mut tx, actor, "pools", request, &rows).await?;
        let pools = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(pool)
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(PoolPage { pools, next_cursor })
    }
    pub async fn create_pool(
        &self,
        actor: &Actor,
        request: &CreatePoolRequest,
    ) -> Result<PoolInspection> {
        require(
            request.staging_bytes <= request.capacity_bytes,
            ErrorCode::InvalidRequest,
        )?;
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let rev = id();
        sqlx::query("INSERT INTO pools VALUES($1,$2,$3)")
            .bind(request.id.as_str())
            .bind(&rev)
            .bind(request.capacity_bytes.get() as i64)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO allocations(id,pool_id,capacity) VALUES($1,$2,$3)")
            .bind(id())
            .bind(request.id.as_str())
            .bind(request.staging_bytes.get() as i64)
            .execute(&mut *tx)
            .await?;
        audit(&mut tx, actor, None, "pool.created", json(request)?).await?;
        tx.commit().await?;
        Ok(PoolInspection {
            id: request.id.clone(),
            revision: rev.parse()?,
            allocatable_bytes: request.capacity_bytes,
            allocated_bytes: request.staging_bytes,
            reported_available_bytes: None,
        })
    }
    pub async fn update_pool(
        &self,
        actor: &Actor,
        pool_id: &PoolId,
        request: &UpdatePoolRequest,
    ) -> Result<PoolInspection> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = sqlx::query("SELECT * FROM pools WHERE id=$1 FOR UPDATE")
            .bind(pool_id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        revision(&row, &request.expected_revision)?;
        let allocated: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(capacity),0)::bigint FROM allocations WHERE pool_id=$1",
        )
        .bind(pool_id.as_str())
        .fetch_one(&mut *tx)
        .await?;
        require(
            request.capacity_bytes.get() >= allocated as u64,
            ErrorCode::QuotaExhausted,
        )?;
        let rev = id();
        sqlx::query("UPDATE pools SET capacity=$2,revision=$3 WHERE id=$1")
            .bind(pool_id.as_str())
            .bind(request.capacity_bytes.get() as i64)
            .bind(&rev)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            actor,
            None,
            "pool.resized",
            serde_json::json!({"pool_id":pool_id,"capacity_bytes":request.capacity_bytes}),
        )
        .await?;
        tx.commit().await?;
        Ok(PoolInspection {
            id: pool_id.clone(),
            revision: rev.parse()?,
            allocatable_bytes: request.capacity_bytes,
            allocated_bytes: (allocated as u64).try_into()?,
            reported_available_bytes: None,
        })
    }
    pub async fn allocation_management(
        &self,
        actor: &Actor,
        allocation_id: &AllocationId,
    ) -> Result<AllocationManagement> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        allocation(
            &sqlx::query("SELECT * FROM allocations WHERE id=$1")
                .bind(allocation_id.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error(ErrorCode::NotFound))?,
        )
    }
    pub async fn create_allocation(
        &self,
        actor: &Actor,
        request: &CreateAllocationRequest,
    ) -> Result<AllocationManagement> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        lock_root(&mut tx, request.root_id.as_str()).await?;
        let row = sqlx::query(
            "INSERT INTO allocations(id,pool_id,root_id,capacity) VALUES($1,$2,$3,$4) RETURNING *",
        )
        .bind(id())
        .bind(request.pool_id.as_str())
        .bind(request.root_id.as_str())
        .bind(request.capacity_bytes.get() as i64)
        .fetch_one(&mut *tx)
        .await?;
        audit(
            &mut tx,
            actor,
            Some(request.root_id.as_str()),
            "allocation.created",
            json(request)?,
        )
        .await?;
        tx.commit().await?;
        allocation(&row)
    }
    pub async fn resize_allocation(
        &self,
        actor: &Actor,
        allocation_id: &AllocationId,
        request: &ResizeAllocationRequest,
    ) -> Result<AllocationManagement> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let row = sqlx::query("SELECT * FROM allocations WHERE id=$1")
            .bind(allocation_id.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        let root: Option<String> = row.get("root_id");
        if let Some(root) = &root {
            crate::content_catalog::lock_content_root(&mut tx, root).await?;
        }
        // Match admission's pool-before-allocation lock order.
        sqlx::query("SELECT id FROM pools WHERE id=$1 FOR UPDATE")
            .bind(row.get::<String, _>("pool_id"))
            .fetch_one(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT * FROM allocations WHERE id=$1 FOR UPDATE")
            .bind(allocation_id.as_str())
            .fetch_one(&mut *tx)
            .await?;
        revision(&row, &request.expected_revision)?;
        if let Some(root) = &root
            && request.capacity_bytes.get() < row.get::<i64, _>("capacity") as u64
        {
            let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE root_id=$1 AND state IN ('active','recovering','cleaning_up'))").bind(root).fetch_one(&mut *tx).await?;
            require(!busy, ErrorCode::RootBusy)?;
        }

        require(
            request.capacity_bytes.get()
                >= (row.get::<i64, _>("reserved") + row.get::<i64, _>("occupied")) as u64,
            ErrorCode::QuotaExhausted,
        )?;
        if let Some(root) = &root {
            let required:i64=sqlx::query_scalar("SELECT r.quota*count(rb.backend_id) FROM roots r LEFT JOIN root_backends rb ON rb.root_id=r.id AND rb.backend_id IN (SELECT id FROM backends WHERE pool_id=$2) WHERE r.id=$1 GROUP BY r.id").bind(root).bind(row.get::<String,_>("pool_id")).fetch_one(&mut *tx).await?;
            require(
                request.capacity_bytes.get() >= required as u64,
                ErrorCode::QuotaExhausted,
            )?;
        }
        let row =
            sqlx::query("UPDATE allocations SET capacity=$2,revision=$3 WHERE id=$1 RETURNING *")
                .bind(allocation_id.as_str())
                .bind(request.capacity_bytes.get() as i64)
                .bind(id())
                .fetch_one(&mut *tx)
                .await?;
        audit(&mut tx,actor,root.as_deref(),"allocation.resized",serde_json::json!({"allocation_id":allocation_id,"capacity_bytes":request.capacity_bytes})).await?;
        tx.commit().await?;
        allocation(&row)
    }
}
