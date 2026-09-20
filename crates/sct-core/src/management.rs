//! Account and delegated management. All mutations share authorization and catalog transactions.
use crate::{
    auth::{self, Actor},
    error::{Error, Result, id, require},
    lease::Writer,
    paging,
    roots::{json, lock_root, revision, root_row},
};
use sct_protocol::*;
use sqlx::{PgConnection, Row, postgres::PgRow};

pub(crate) async fn audit(
    db: &mut PgConnection,
    actor: &Actor,
    root: Option<&str>,
    event: &str,
    details: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO management_audit(root_id,principal_id,event,details) VALUES($1,$2,$3,$4)",
    )
    .bind(root)
    .bind(actor.principal_id())
    .bind(event)
    .bind(details)
    .execute(db)
    .await?;
    Ok(())
}
fn principal(row: &PgRow) -> Result<PrincipalInspection> {
    Ok(PrincipalInspection {
        id: row.get::<String, _>("id").parse()?,
        administrator: row.get("administrator"),
        disabled: row.get("disabled"),
        revision: row.get::<String, _>("revision").parse()?,
    })
}
async fn account_access(db: &mut PgConnection, actor: &Actor, target: &PrincipalId) -> Result<()> {
    require(
        auth::administrator(db, actor).await? || actor.principal_id() == target.as_str(),
        ErrorCode::Forbidden,
    )
}
async fn root_management(
    db: &mut PgConnection,
    actor: &Actor,
    root: &RootId,
) -> Result<RootManagement> {
    let capabilities = auth::capabilities(db, actor, root.as_str()).await?;
    require(!capabilities.is_empty(), ErrorCode::Forbidden)?;
    let row = sqlx::query("SELECT *, (extract(epoch FROM last_commit_at)*1000)::bigint AS last_commit_ms FROM roots WHERE id=$1")
        .bind(root.as_str()).fetch_one(db).await?;
    Ok(RootManagement {
        root_id: root.clone(),
        revision: row.get::<String, _>("revision").parse()?,
        quota_bytes: (row.get::<i64, _>("quota") as u64).try_into()?,
        quota_ceiling_bytes: (row.get::<i64, _>("quota_ceiling") as u64).try_into()?,
        min_versions: (row.get::<i64, _>("min_versions") as u64).try_into()?,
        max_versions: (row.get::<i64, _>("max_versions") as u64).try_into()?,
        allow_relaxed: row.get("allow_relaxed"),
        expected_upload_interval_seconds: row
            .get::<Option<i64>, _>("expected_upload_interval_seconds")
            .map(|n| ByteCount::try_from(n as u64))
            .transpose()?,
        last_commit_at_ms: row
            .get::<Option<i64>, _>("last_commit_ms")
            .map(|n| TimestampMs::try_from(n as u64))
            .transpose()?,
        capabilities,
    })
}
impl Writer {
    pub async fn account(&self, actor: &Actor) -> Result<AccountInspection> {
        let mut tx = self.transaction().await?;
        Ok(AccountInspection {
            principal_id: actor.principal_id().parse()?,
            administrator: auth::administrator(&mut tx, actor).await?,
        })
    }
    pub async fn inspect_principal(
        &self,
        actor: &Actor,
        target: &PrincipalId,
    ) -> Result<PrincipalInspection> {
        let mut tx = self.transaction().await?;
        account_access(&mut tx, actor, target).await?;
        principal(
            &sqlx::query("SELECT * FROM principals WHERE id=$1")
                .bind(target.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error(ErrorCode::NotFound))?,
        )
    }
    pub async fn list_principals(
        &self,
        actor: &Actor,
        request: &ListRequest,
    ) -> Result<PrincipalPage> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        let scope = "principals";
        let binding = json(&request.limit)?;
        let after: String = match &request.cursor {
            Some(token) => paging::consume(&mut tx, actor, scope, token, &binding).await?,
            None => String::new(),
        };
        let rows = sqlx::query("SELECT * FROM principals WHERE id>$1 ORDER BY id LIMIT $2")
            .bind(after)
            .bind(request.limit.get() as i64 + 1)
            .fetch_all(&mut *tx)
            .await?;
        let more = rows.len() > request.limit.get() as usize;
        let principals = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(principal)
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    scope,
                    binding,
                    json(&principals.last().unwrap().id)?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(PrincipalPage {
            principals,
            next_cursor,
        })
    }
    pub async fn update_principal(
        &self,
        actor: &Actor,
        target: &PrincipalId,
        request: &UpdatePrincipalRequest,
    ) -> Result<PrincipalInspection> {
        let mut tx = self.transaction().await?;
        // Serialize role changes before acquiring authentication's principal locks.
        sqlx::query("SELECT pg_advisory_xact_lock(73628491)")
            .execute(&mut *tx)
            .await?;
        auth::admin(&mut tx, actor).await?;
        let row = sqlx::query("SELECT * FROM principals WHERE id=$1 FOR UPDATE")
            .bind(target.as_str())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        revision(&row, &request.expected_revision)?;
        if row.get::<bool, _>("administrator")
            && !row.get::<bool, _>("disabled")
            && (!request.administrator || request.disabled)
        {
            let others: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM principals WHERE administrator AND NOT disabled AND id<>$1",
            )
            .bind(target.as_str())
            .fetch_one(&mut *tx)
            .await?;
            require(others > 0, ErrorCode::InvalidRequest)?;
        }
        let row = sqlx::query("UPDATE principals SET administrator=$2,disabled=$3,revision=$4 WHERE id=$1 RETURNING *").bind(target.as_str()).bind(request.administrator).bind(request.disabled).bind(id()).fetch_one(&mut *tx).await?;
        audit(&mut tx,actor,None,"principal.updated",serde_json::json!({"principal_id":target,"administrator":request.administrator,"disabled":request.disabled})).await?;
        tx.commit().await?;
        principal(&row)
    }
    pub async fn list_credentials(
        &self,
        actor: &Actor,
        target: &PrincipalId,
        request: &ListRequest,
    ) -> Result<CredentialPage> {
        let mut tx = self.transaction().await?;
        account_access(&mut tx, actor, target).await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM principals WHERE id=$1)")
                .bind(target.as_str())
                .fetch_one(&mut *tx)
                .await?;
        require(exists, ErrorCode::NotFound)?;
        let scope = format!("credentials:{}", target.as_str());
        let binding = json(&request.limit)?;
        let after: String = match &request.cursor {
            Some(token) => paging::consume(&mut tx, actor, &scope, token, &binding).await?,
            None => String::new(),
        };
        let rows = sqlx::query("SELECT id,principal_id,kind,revoked,(extract(epoch FROM expires_at)*1000)::bigint AS expires_ms FROM credentials WHERE principal_id=$1 AND id>$2 ORDER BY id LIMIT $3").bind(target.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let credentials = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|row| {
                Ok(CredentialInspection {
                    id: row.get::<String, _>("id").parse()?,
                    principal_id: target.clone(),
                    kind: row.get("kind"),
                    revoked: row.get("revoked"),
                    expires_at_ms: row
                        .get::<Option<i64>, _>("expires_ms")
                        .map(|n| TimestampMs::try_from(n as u64))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if more {
            Some(
                paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    binding,
                    json(&credentials.last().unwrap().id)?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(CredentialPage {
            credentials,
            next_cursor,
        })
    }
    pub async fn root_management(&self, actor: &Actor, root: &RootId) -> Result<RootManagement> {
        let mut tx = self.transaction().await?;
        root_management(&mut tx, actor, root).await
    }
    pub async fn update_root_management(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &UpdateRootManagementRequest,
    ) -> Result<RootManagement> {
        request.validate()?;
        let mut tx = self.transaction().await?;
        auth::authorize(&mut tx, actor, root.as_str(), Capability::ManageRoot).await?;
        lock_root(&mut tx, root.as_str()).await?;
        auth::authorize(&mut tx, actor, root.as_str(), Capability::ManageRoot).await?;
        let row = root_row(&mut tx, root.as_str()).await?;
        revision(&row, &request.expected_revision)?;
        require(
            request.quota_bytes.get() <= row.get::<i64, _>("quota_ceiling") as u64,
            ErrorCode::QuotaExhausted,
        )?;
        // A ceiling is authorization, not proof of physical capacity.
        let capacities = sqlx::query("SELECT a.capacity,count(*) AS copies FROM root_backends rb JOIN backends b ON b.id=rb.backend_id JOIN allocations a ON a.pool_id=b.pool_id AND a.root_id=rb.root_id WHERE rb.root_id=$1 GROUP BY a.id,a.capacity").bind(root.as_str()).fetch_all(&mut *tx).await?;
        for capacity in capacities {
            require(
                request
                    .quota_bytes
                    .get()
                    .checked_mul(capacity.get::<i64, _>("copies") as u64)
                    .is_some_and(|n| n <= capacity.get::<i64, _>("capacity") as u64),
                ErrorCode::QuotaExhausted,
            )?;
        }
        sqlx::query("UPDATE roots SET display_name=$2,quota=$3,expected_upload_interval_seconds=$4,revision=$5 WHERE id=$1").bind(root.as_str()).bind(request.display_name.as_str()).bind(request.quota_bytes.get() as i64).bind(request.expected_upload_interval_seconds.map(|n|n.get() as i64)).bind(id()).execute(&mut *tx).await?;
        audit(&mut tx,actor,Some(root.as_str()),"root.settings_updated",serde_json::json!({"quota_bytes":request.quota_bytes,"expected_upload_interval_seconds":request.expected_upload_interval_seconds})).await?;
        let result = root_management(&mut tx, actor, root).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn update_quota_ceiling(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &UpdateQuotaCeilingRequest,
    ) -> Result<RootManagement> {
        let mut tx = self.transaction().await?;
        auth::admin(&mut tx, actor).await?;
        lock_root(&mut tx, root.as_str()).await?;
        let row = root_row(&mut tx, root.as_str()).await?;
        revision(&row, &request.expected_revision)?;
        require(
            request.quota_ceiling_bytes.get() >= row.get::<i64, _>("quota") as u64,
            ErrorCode::InvalidRequest,
        )?;
        sqlx::query("UPDATE roots SET quota_ceiling=$2,revision=$3 WHERE id=$1")
            .bind(root.as_str())
            .bind(request.quota_ceiling_bytes.get() as i64)
            .bind(id())
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            actor,
            Some(root.as_str()),
            "root.ceiling_updated",
            serde_json::json!({"quota_ceiling_bytes":request.quota_ceiling_bytes}),
        )
        .await?;
        let result = root_management(&mut tx, actor, root).await?;
        tx.commit().await?;
        Ok(result)
    }
}
