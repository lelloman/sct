use crate::{
    auth::{Actor, admin, administrator, capabilities},
    backend::Registry,
    error::{Error, Result, id, require},
    lease::Writer,
};
use sct_protocol::*;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::collections::BTreeMap;

pub struct PoolSetup {
    pub id: PoolId,
    pub capacity: ByteCount,
    pub staging: ByteCount,
}
pub struct BackendSetup {
    pub pool_id: PoolId,
    pub inspection: BackendInspection,
    pub settings: Value,
}
pub(crate) fn json<T: serde::Serialize>(value: &T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}
pub(crate) fn parse<T: serde::de::DeserializeOwned>(row: &PgRow, column: &str) -> Result<T> {
    Ok(serde_json::from_value(row.get(column))?)
}
pub(crate) async fn root_row(db: &mut PgConnection, root: &str) -> Result<PgRow> {
    sqlx::query("SELECT * FROM roots WHERE id=$1")
        .bind(root)
        .fetch_optional(db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))
}
pub(crate) async fn lock_root(db: &mut PgConnection, root: &str) -> Result<()> {
    let exists = sqlx::query("SELECT id FROM roots WHERE id=$1 FOR UPDATE")
        .bind(root)
        .fetch_optional(&mut *db)
        .await?;
    require(exists.is_some(), ErrorCode::NotFound)?;
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE root_id=$1 AND state IN ('active','recovering','cleaning_up'))").bind(root).fetch_one(db).await?;
    require(!busy, ErrorCode::RootBusy)
}
pub(crate) fn revision(row: &PgRow, expected: &Revision) -> Result<()> {
    require(
        row.get::<String, _>("revision") == expected.as_str(),
        ErrorCode::RevisionConflict,
    )
}
fn members(placement: &Placement) -> Vec<&BackendId> {
    match placement {
        Placement::Single { backend_id, .. } => vec![backend_id],
        Placement::Mirror { backend_ids, .. } => backend_ids.iter().collect(),
    }
}
async fn placement_pools(
    db: &mut PgConnection,
    placement: &Placement,
) -> Result<BTreeMap<String, u64>> {
    placement.validate()?;
    let mut pools = BTreeMap::new();
    for backend in members(placement) {
        let pool: String = sqlx::query_scalar("SELECT pool_id FROM backends WHERE id=$1")
            .bind(backend.as_str())
            .fetch_optional(&mut *db)
            .await?
            .ok_or(Error(ErrorCode::InvalidRequest))?;
        *pools.entry(pool).or_default() += 1;
    }
    Ok(pools)
}
async fn inspect(db: &mut PgConnection, actor: &Actor, root: &str) -> Result<RootInspection> {
    let capabilities = capabilities(db, actor, root).await?;
    require(!capabilities.is_empty(), ErrorCode::NotFound)?;
    let row = root_row(db, root).await?;
    Ok(RootInspection {
        id: root.parse()?,
        owner_id: row.get::<String, _>("owner_id").parse()?,
        display_name: row.get::<String, _>("display_name").parse()?,
        mount_path: row.get::<String, _>("mount_path").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        state: serde_json::from_value(json!(row.get::<String, _>("state")))?,
        placement: parse(&row, "placement")?,
        quota_bytes: ByteCount::try_from(row.get::<i64, _>("quota") as u64)?,
        default_policy: parse(&row, "default_policy")?,
        capabilities,
    })
}
pub(crate) async fn replay<T: serde::de::DeserializeOwned>(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    key: &IdempotencyKey,
    request: &Value,
) -> Result<Option<T>> {
    // Serializes retries even when the result (e.g. a new root) has no row yet.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{}:{scope}:{}", actor.principal, key.as_str()))
        .execute(&mut *db)
        .await?;
    if let Some(row) = sqlx::query(
        "SELECT request,response FROM idempotency WHERE principal_id=$1 AND scope=$2 AND key=$3",
    )
    .bind(&actor.principal)
    .bind(scope)
    .bind(key.as_str())
    .fetch_optional(db)
    .await?
    {
        require(
            row.get::<Value, _>("request") == *request,
            ErrorCode::IdempotencyConflict,
        )?;
        return Ok(Some(parse(&row, "response")?));
    }
    Ok(None)
}
pub(crate) async fn remember(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    key: &IdempotencyKey,
    request: Value,
    response: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO idempotency VALUES($1,$2,$3,$4,$5)")
        .bind(&actor.principal)
        .bind(scope)
        .bind(key.as_str())
        .bind(request)
        .bind(response)
        .execute(db)
        .await?;
    Ok(())
}
impl Writer {
    /// Startup-only infrastructure reconciliation under the exclusive writer lease.
    pub async fn configure(
        &self,
        registry: &Registry,
        pools: &[PoolSetup],
        backends: &[BackendSetup],
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let configured: Vec<String> = backends
            .iter()
            .map(|b| b.inspection.id.as_str().to_owned())
            .collect();
        let missing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM root_backends WHERE NOT backend_id=ANY($1))",
        )
        .bind(&configured)
        .fetch_one(&mut *tx)
        .await?;
        require(!missing, ErrorCode::InvalidRequest)?;
        // Deterministic pool lock order prevents cross-pool reservation deadlocks.
        let mut sorted: Vec<_> = pools.iter().collect();
        sorted.sort_by_key(|p| p.id.as_str());
        for pool in sorted {
            sqlx::query("INSERT INTO pools VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET capacity=EXCLUDED.capacity,revision=EXCLUDED.revision")
              .bind(pool.id.as_str()).bind(id()).bind(pool.capacity.get() as i64).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO allocations(id,pool_id,capacity) VALUES($1,$2,$3) ON CONFLICT(pool_id,root_id) DO UPDATE SET capacity=EXCLUDED.capacity")
              .bind(id()).bind(pool.id.as_str()).bind(pool.staging.get() as i64).execute(&mut *tx).await?;
        }
        for backend in backends {
            let identity = registry.validate(&backend.inspection, &backend.settings)?;
            let existing =
                sqlx::query("SELECT destination_identity,kind,pool_id FROM backends WHERE id=$1")
                    .bind(backend.inspection.id.as_str())
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some(row) = existing {
                // Destination identity and pool membership cannot be changed through config.
                require(
                    row.get::<String, _>("destination_identity") == identity
                        && row.get::<String, _>("kind") == backend.inspection.adapter_kind
                        && row.get::<String, _>("pool_id") == backend.pool_id.as_str(),
                    ErrorCode::InvalidRequest,
                )?;
            }
            let mut inspection = backend.inspection.clone();
            inspection.observed_health = registry
                .get(&inspection.adapter_kind)?
                .probe(&backend.settings);
            inspection.last_observed_at_ms = None;
            sqlx::query("INSERT INTO backends VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET inspection=EXCLUDED.inspection")
              .bind(inspection.id.as_str()).bind(backend.pool_id.as_str()).bind(&inspection.adapter_kind).bind(identity).bind(json(&inspection)?).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM backends WHERE NOT id=ANY($1)")
            .bind(&configured)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn create_root(
        &self,
        actor: &Actor,
        request: &CreateRootRequest,
    ) -> Result<RootInspection> {
        request.validate()?;
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        if let Some(result) = replay(
            &mut tx,
            actor,
            "roots",
            &request.idempotency_key,
            &json(request)?,
        )
        .await?
        {
            return Ok(result);
        }
        // Owner lock serializes overlapping mount checks, including concurrent creates.
        let owner =
            sqlx::query("SELECT id FROM principals WHERE id=$1 AND NOT disabled FOR UPDATE")
                .bind(request.owner_id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        require(owner.is_some(), ErrorCode::InvalidRequest)?;
        let mounts: Vec<String> =
            sqlx::query_scalar("SELECT mount_path FROM roots WHERE owner_id=$1")
                .bind(request.owner_id.as_str())
                .fetch_all(&mut *tx)
                .await?;
        let mount = request.mount_path.as_str();
        require(
            !mounts.iter().any(|m| paths_overlap(m, mount)),
            ErrorCode::InvalidRequest,
        )?;
        let pools = placement_pools(&mut tx, &request.placement).await?;
        require(
            pools.len() == request.allocations.len(),
            ErrorCode::InvalidRequest,
        )?;
        for allocation in &request.allocations {
            let copies = pools
                .get(allocation.pool_id.as_str())
                .ok_or(Error(ErrorCode::InvalidRequest))?;
            require(
                request
                    .quota_bytes
                    .get()
                    .checked_mul(*copies)
                    .is_some_and(|n| n <= allocation.retained_capacity_bytes.get()),
                ErrorCode::QuotaExhausted,
            )?;
        }
        let root = id();
        let policy = id();
        sqlx::query("INSERT INTO roots(id,owner_id,display_name,mount_path,revision,state,placement,quota,default_policy,policy_id,policy_revision) VALUES($1,$2,$3,$4,$5,'disabled',$6,$7,$8,$9,$10)")
          .bind(&root).bind(request.owner_id.as_str()).bind(request.display_name.as_str()).bind(mount).bind(id()).bind(json(&request.placement)?).bind(request.quota_bytes.get() as i64).bind(json(&request.default_policy)?).bind(policy).bind(id()).execute(&mut *tx).await?;
        for backend in members(&request.placement) {
            sqlx::query("INSERT INTO root_backends VALUES($1,$2)")
                .bind(&root)
                .bind(backend.as_str())
                .execute(&mut *tx)
                .await?;
        }
        let mut allocations: Vec<_> = request.allocations.iter().collect();
        allocations.sort_by_key(|a| a.pool_id.as_str());
        for a in allocations {
            sqlx::query("INSERT INTO allocations(id,pool_id,root_id,capacity) VALUES($1,$2,$3,$4)")
                .bind(id())
                .bind(a.pool_id.as_str())
                .bind(&root)
                .bind(a.retained_capacity_bytes.get() as i64)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO entries(id,root_id,name,kind,revision) VALUES($1,$2,$3,'directory',$4)",
        )
        .bind(id())
        .bind(&root)
        .bind(request.display_name.as_str())
        .bind(id())
        .execute(&mut *tx)
        .await?;
        let result = inspect(&mut tx, actor, &root).await?;
        remember(
            &mut tx,
            actor,
            "roots",
            &request.idempotency_key,
            json(request)?,
            json(&result)?,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn inspect_root(&self, actor: &Actor, root: &RootId) -> Result<RootInspection> {
        let mut tx = self.transaction().await?;
        inspect(&mut tx, actor, root.as_str()).await
    }
    pub async fn list_roots(&self, actor: &Actor, request: &ListRequest) -> Result<RootPage> {
        let mut tx = self.transaction().await?;
        let is_admin = administrator(&mut tx, actor).await?;
        let binding = json!({"limit":request.limit});
        let after: String = if let Some(cursor) = &request.cursor {
            crate::paging::consume(&mut tx, actor, "roots", cursor, &binding).await?
        } else {
            String::new()
        };
        let rows:Vec<String>=sqlx::query_scalar("SELECT r.id FROM roots r WHERE r.id>$1 AND ($2 OR r.owner_id=$3 OR EXISTS(SELECT 1 FROM grants g WHERE g.root_id=r.id AND g.principal_id=$3 AND g.capabilities<>'[]'::jsonb)) ORDER BY r.id LIMIT $4")
          .bind(after).bind(is_admin).bind(&actor.principal).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let mut items = Vec::new();
        for root in rows.iter().take(request.limit.get() as usize) {
            items.push(inspect(&mut tx, actor, root).await?);
        }
        let next_cursor = if more {
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    "roots",
                    binding,
                    json!(items.last().unwrap().id),
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(RootPage { items, next_cursor })
    }
    pub async fn update_root(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &UpdateRootRequest,
    ) -> Result<RootInspection> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        lock_root(&mut tx, root.as_str()).await?;
        let row = root_row(&mut tx, root.as_str()).await?;
        revision(&row, &request.expected_revision)?;
        require(
            request.state != RootState::Recovering,
            ErrorCode::InvalidRequest,
        )?;
        let existing: Placement = parse(&row, "placement")?;
        if row.get::<bool, _>("populated") {
            require(
                members(&existing) == members(&request.placement),
                ErrorCode::InvalidRequest,
            )?;
        }
        let pools = placement_pools(&mut tx, &request.placement).await?;
        for (pool, copies) in &pools {
            let capacity: Option<i64> = sqlx::query_scalar(
                "SELECT capacity FROM allocations WHERE root_id=$1 AND pool_id=$2",
            )
            .bind(root.as_str())
            .bind(pool)
            .fetch_optional(&mut *tx)
            .await?;
            require(
                capacity.is_some_and(|n| {
                    request
                        .quota_bytes
                        .get()
                        .checked_mul(*copies)
                        .is_some_and(|required| required <= n as u64)
                }),
                ErrorCode::QuotaExhausted,
            )?;
        }
        sqlx::query("UPDATE roots SET display_name=$2,state=$3,placement=$4,quota=$5,revision=$6 WHERE id=$1")
          .bind(root.as_str()).bind(request.display_name.as_str()).bind(json(&request.state)?.as_str().unwrap()).bind(json(&request.placement)?).bind(request.quota_bytes.get() as i64).bind(id()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM root_backends WHERE root_id=$1")
            .bind(root.as_str())
            .execute(&mut *tx)
            .await?;
        for backend in members(&request.placement) {
            sqlx::query("INSERT INTO root_backends VALUES($1,$2)")
                .bind(root.as_str())
                .bind(backend.as_str())
                .execute(&mut *tx)
                .await?;
        }
        let result = inspect(&mut tx, actor, root.as_str()).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn inspect_pool(&self, actor: &Actor, pool: &PoolId) -> Result<PoolInspection> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        let row=sqlx::query("SELECT p.*,COALESCE((SELECT sum(capacity)::bigint FROM allocations WHERE pool_id=p.id),0) allocated FROM pools p WHERE id=$1").bind(pool.as_str()).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        Ok(PoolInspection {
            id: pool.clone(),
            revision: row.get::<String, _>("revision").parse()?,
            allocatable_bytes: ByteCount::try_from(row.get::<i64, _>("capacity") as u64)?,
            allocated_bytes: ByteCount::try_from(row.get::<i64, _>("allocated") as u64)?,
            reported_available_bytes: None,
        })
    }
    pub async fn inspect_allocation(
        &self,
        actor: &Actor,
        allocation: &AllocationId,
    ) -> Result<AllocationInspection> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        let row = sqlx::query("SELECT * FROM allocations WHERE id=$1")
            .bind(allocation.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok(AllocationInspection {
            id: allocation.clone(),
            pool_id: row.get::<String, _>("pool_id").parse()?,
            owner: match row.get::<Option<String>, _>("root_id") {
                Some(root) => AllocationOwner::Root {
                    root_id: root.parse()?,
                },
                None => AllocationOwner::ServerStaging,
            },
            capacity_bytes: ByteCount::try_from(row.get::<i64, _>("capacity") as u64)?,
            reserved_bytes: ByteCount::try_from(row.get::<i64, _>("reserved") as u64)?,
            occupied_bytes: ByteCount::try_from(row.get::<i64, _>("occupied") as u64)?,
            pending_cleanup_bytes: ByteCount::try_from(row.get::<i64, _>("pending_cleanup") as u64)?,
        })
    }
    pub async fn inspect_backend(
        &self,
        actor: &Actor,
        backend: &BackendId,
    ) -> Result<BackendInspection> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        let value: Value = sqlx::query_scalar("SELECT inspection FROM backends WHERE id=$1")
            .bind(backend.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        Ok(serde_json::from_value(value)?)
    }
    pub async fn require_admin(&self, actor: &Actor) -> Result<()> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await
    }
}
fn paths_overlap(a: &str, b: &str) -> bool {
    a == "/"
        || b == "/"
        || a == b
        || a.strip_prefix(b).is_some_and(|s| s.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|s| s.starts_with('/'))
}

impl Writer {
    pub async fn list_allocations(
        &self,
        actor: &Actor,
        request: &ListRequest,
    ) -> Result<AllocationPage> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        let binding = json(&request.limit)?;
        let after: String = if let Some(token) = &request.cursor {
            crate::paging::consume(&mut tx, actor, "allocations", token, &binding).await?
        } else {
            String::new()
        };
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT id FROM allocations WHERE id>$1 ORDER BY id LIMIT $2")
                .bind(after)
                .bind(request.limit.get() as i64 + 1)
                .fetch_all(&mut *tx)
                .await?;
        let more = rows.len() > request.limit.get() as usize;
        let next_cursor = if more {
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    "allocations",
                    binding,
                    json(&rows[request.limit.get() as usize - 1])?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        let mut items = Vec::new();
        for row in rows.iter().take(request.limit.get() as usize) {
            items.push(self.inspect_allocation(actor, &row.parse()?).await?);
        }
        Ok(AllocationPage { items, next_cursor })
    }
}
