use crate::{
    auth::{Actor, authorize, capabilities},
    content_catalog::lock_content_root as lock_root,
    error::{Error, Result, id, require},
    lease::Writer,
    roots::{json, parse, revision},
};
use db::{PgConnection, PgRow};
use sct_protocol::*;
use simple_server::database::postgres as db;
fn inspect(row: &PgRow) -> Result<GrantInspection> {
    Ok(GrantInspection {
        id: row.get::<String, _>("id").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        principal_id: row.get::<String, _>("principal_id").parse()?,
        revision: row.get::<String, _>("revision").parse()?,
        capabilities: parse(row, "capabilities")?,
    })
}
async fn grant_row(db: &mut PgConnection, grant: &GrantId) -> Result<PgRow> {
    db::query("SELECT * FROM grants WHERE id=$1")
        .bind(grant.as_str())
        .fetch_optional(db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))
}
async fn check(
    db: &mut PgConnection,
    actor: &Actor,
    root: &str,
    requested: &[Capability],
) -> Result<()> {
    authorize(db, actor, root, Capability::ManageGrants).await?;
    let allowed = capabilities(db, actor, root).await?;
    require(
        requested.iter().all(|c| allowed.contains(c)),
        ErrorCode::Forbidden,
    )?;
    require(
        requested
            .iter()
            .enumerate()
            .all(|(i, c)| !requested[..i].contains(c)),
        ErrorCode::InvalidRequest,
    )
}
impl Writer {
    pub async fn create_grant(
        &self,
        actor: &Actor,
        request: &CreateGrantRequest,
    ) -> Result<GrantInspection> {
        let mut tx = self.transaction().await?;
        check(
            &mut tx,
            actor,
            request.root_id.as_str(),
            &request.capabilities,
        )
        .await?;
        lock_root(&mut tx, request.root_id.as_str()).await?;
        check(
            &mut tx,
            actor,
            request.root_id.as_str(),
            &request.capabilities,
        )
        .await?;
        let row = db::query("INSERT INTO grants VALUES($1,$2,$3,$4,$5) RETURNING *")
            .bind(id())
            .bind(request.root_id.as_str())
            .bind(request.principal_id.as_str())
            .bind(id())
            .bind(json(&request.capabilities)?)
            .fetch_one(&mut *tx)
            .await?;
        crate::management::audit(
            &mut tx,
            actor,
            Some(&row.get::<String, _>("root_id")),
            "grant.created",
            json(&inspect(&row)?)?,
        )
        .await?;
        tx.commit().await?;
        inspect(&row)
    }
    pub async fn inspect_grant(&self, actor: &Actor, grant: &GrantId) -> Result<GrantInspection> {
        let mut tx = self.transaction().await?;
        let row = grant_row(&mut tx, grant).await?;
        authorize(
            &mut tx,
            actor,
            &row.get::<String, _>("root_id"),
            Capability::ManageGrants,
        )
        .await?;
        inspect(&row)
    }
    pub async fn update_grant(
        &self,
        actor: &Actor,
        grant: &GrantId,
        request: &UpdateGrantRequest,
    ) -> Result<GrantInspection> {
        let mut tx = self.transaction().await?;
        let row = grant_row(&mut tx, grant).await?;
        let root: String = row.get("root_id");
        check(&mut tx, actor, &root, &request.capabilities).await?;
        lock_root(&mut tx, &root).await?;
        check(&mut tx, actor, &root, &request.capabilities).await?;
        let row = grant_row(&mut tx, grant).await?;
        revision(&row, &request.expected_revision)?;
        let row =
            db::query("UPDATE grants SET revision=$2,capabilities=$3 WHERE id=$1 RETURNING *")
                .bind(grant.as_str())
                .bind(id())
                .bind(json(&request.capabilities)?)
                .fetch_one(&mut *tx)
                .await?;
        crate::management::audit(
            &mut tx,
            actor,
            Some(&row.get::<String, _>("root_id")),
            "grant.updated",
            json(&inspect(&row)?)?,
        )
        .await?;
        tx.commit().await?;
        inspect(&row)
    }
    pub async fn revoke_grant(
        &self,
        actor: &Actor,
        grant: &GrantId,
        expected: &Revision,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        let row = grant_row(&mut tx, grant).await?;
        let root: String = row.get("root_id");
        authorize(&mut tx, actor, &root, Capability::ManageGrants).await?;
        lock_root(&mut tx, &root).await?;
        authorize(&mut tx, actor, &root, Capability::ManageGrants).await?;
        revision(&grant_row(&mut tx, grant).await?, expected)?;
        db::query("DELETE FROM grants WHERE id=$1")
            .bind(grant.as_str())
            .execute(&mut *tx)
            .await?;
        crate::management::audit(
            &mut tx,
            actor,
            Some(&root),
            "grant.revoked",
            serde_json::json!({"grant_id":grant}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn list_grants(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &ListRequest,
    ) -> Result<GrantPage> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManageGrants).await?;
        let scope = format!("grants:{}", root.as_str());
        let binding = json(&request.limit)?;
        let after: String = if let Some(token) = &request.cursor {
            crate::paging::consume(&mut tx, actor, &scope, token, &binding).await?
        } else {
            String::new()
        };
        let rows =
            db::query("SELECT * FROM grants WHERE root_id=$1 AND id>$2 ORDER BY id LIMIT $3")
                .bind(root.as_str())
                .bind(after)
                .bind(request.limit.get() as i64 + 1)
                .fetch_all(&mut *tx)
                .await?;
        let more = rows.len() > request.limit.get() as usize;
        let items: Vec<_> = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(inspect)
            .collect::<Result<_>>()?;
        let next_cursor = if more {
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    binding,
                    json(&items.last().unwrap().id)?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(GrantPage { items, next_cursor })
    }
}
