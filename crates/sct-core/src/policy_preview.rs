//! Optimistic previews scan bounded version batches without owning the root.
use crate::{
    auth::{Actor, authorize},
    content_catalog::lock_content_root,
    error::{Error, Result, id, require},
    lease::Writer,
    policy::{effective, protected, scope_parts},
    roots::{json, root_row},
};
use db::{PgConnection, PgRow};
use sct_protocol::*;
use simple_server::database::postgres as db;

pub(crate) async fn authority(
    db: &mut PgConnection,
    actor: &Actor,
    root: &str,
) -> Result<serde_json::Value> {
    let capabilities = crate::auth::capabilities(db, actor, root).await?;
    let grant: Option<String> =
        db::query_scalar("SELECT revision FROM grants WHERE root_id=$1 AND principal_id=$2")
            .bind(root)
            .bind(&actor.principal)
            .fetch_optional(db)
            .await?;
    Ok(
        serde_json::json!({"principal":actor.principal,"credential":actor.credential,"capabilities":capabilities,"grant_revision":grant}),
    )
}

pub(crate) async fn validate_change(
    db: &mut PgConnection,
    root: &str,
    change: &PolicyChange,
) -> Result<RetentionPolicy> {
    let (scope, revision, proposed) = match change {
        PolicyChange::Set {
            scope,
            expected_policy_revision,
            policy,
        } => (scope, expected_policy_revision, Some(policy)),
        PolicyChange::Inherit {
            scope,
            expected_policy_revision,
        } => (scope, expected_policy_revision, None),
        PolicyChange::Move { .. } => {
            return Ok(crate::namespace_moves::validate(db, root, change).await?.0);
        }
    };
    let (scope_root, entry) = scope_parts(scope);
    require(scope_root.as_str() == root, ErrorCode::InvalidRequest)?;
    let old = effective(db, root, entry.map(EntryId::as_str)).await?;
    require(&old.revision == revision, ErrorCode::RevisionConflict)?;
    let policy = if let Some(entry) = entry {
        let row = db::query("SELECT parent_id,policy FROM entries WHERE root_id=$1 AND id=$2")
            .bind(root)
            .bind(entry.as_str())
            .fetch_optional(&mut *db)
            .await?
            .ok_or(Error(ErrorCode::NotFound))?;
        let parent: Option<String> = row.get("parent_id");
        let inherited = effective(db, root, parent.as_deref()).await?;
        require(
            inherited.versioning != VersioningMode::AtomicDirectory,
            ErrorCode::InvalidRequest,
        )?;
        if let Some(proposed) = proposed {
            proposed.clone()
        } else {
            require(
                row.get::<Option<serde_json::Value>, _>("policy").is_some(),
                ErrorCode::InvalidRequest,
            )?;
            inherited.policy
        }
    } else {
        proposed.cloned().ok_or(Error(ErrorCode::InvalidRequest))?
    };
    let row = root_row(db, root).await?;
    require(
        policy.keep_versions.get() >= row.get::<i64, _>("min_versions") as u64
            && policy.keep_versions.get() <= row.get::<i64, _>("max_versions") as u64
            && (policy.mode == RetentionMode::Strict || row.get::<bool, _>("allow_relaxed")),
        ErrorCode::Forbidden,
    )?;
    Ok(policy)
}

pub(crate) async fn check_preview(
    db: &mut PgConnection,
    actor: &Actor,
    row: &PgRow,
) -> Result<PreviewChangeRequest> {
    let root: String = row.get("root_id");
    authorize(db, actor, &root, Capability::ManagePolicy).await?;
    require(
        row.get::<String, _>("principal_id") == actor.principal,
        ErrorCode::NotFound,
    )?;
    require(!row.get::<bool, _>("expired"), ErrorCode::RevisionConflict)?;
    let current = root_row(db, &root).await?;
    require(
        current.get::<String, _>("revision") == row.get::<String, _>("root_revision")
            && current.get::<i64, _>("mutation_serial") == row.get::<i64, _>("mutation_serial"),
        ErrorCode::RevisionConflict,
    )?;
    require(
        authority(db, actor, &root).await? == row.get::<serde_json::Value, _>("authority"),
        ErrorCode::RevisionConflict,
    )?;
    let request: PreviewChangeRequest = serde_json::from_value(row.get("request"))?;
    if matches!(request.change, PolicyChange::Move { .. }) {
        authorize(db, actor, &root, Capability::Write).await?;
    }
    validate_change(db, &root, &request.change).await?;
    Ok(request)
}
pub(crate) async fn load(db: &mut PgConnection, root: &str, preview: &str) -> Result<PgRow> {
    db::query("SELECT *,expires_at<=clock_timestamp() expired,(extract(epoch FROM expires_at)*1000)::bigint expires_ms FROM change_previews WHERE root_id=$1 AND id=$2")
        .bind(root).bind(preview).fetch_optional(db).await?.ok_or(Error(ErrorCode::NotFound))
}
fn inspect(row: &PgRow) -> Result<ChangePreview> {
    let request: PreviewChangeRequest = serde_json::from_value(row.get("request"))?;
    Ok(ChangePreview {
        id: row.get::<String, _>("id").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        root_revision: row.get::<String, _>("root_revision").parse()?,
        state: serde_json::from_value(json(&row.get::<String, _>("state"))?)?,
        versions_to_retire: (row.get::<i64, _>("retire_count") as u64).try_into()?,
        versions_losing_protection: (row.get::<i64, _>("loss_count") as u64).try_into()?,
        estimated_payload_bytes: (row.get::<i64, _>("estimated_bytes") as u64).try_into()?,
        expires_at_ms: (row.get::<i64, _>("expires_ms") as u64).try_into()?,
        change: request.change,
    })
}

async fn affected(
    db: &mut PgConnection,
    root: &str,
    entry: &str,
    change: &PolicyChange,
    old: &EffectivePolicy,
) -> Result<bool> {
    let scope = match change {
        PolicyChange::Set { scope, .. } | PolicyChange::Inherit { scope, .. } => scope,
        PolicyChange::Move { entry_id, .. } => {
            let ancestors=db::query("WITH RECURSIVE a AS (SELECT id,parent_id,policy,0 depth FROM entries WHERE root_id=$1 AND id=$2 UNION ALL SELECT e.id,e.parent_id,e.policy,a.depth+1 FROM entries e JOIN a ON e.id=a.parent_id WHERE e.root_id=$1 AND a.depth<4096) SELECT id,policy FROM a ORDER BY depth")
                .bind(root).bind(entry).fetch_all(db).await?;
            for ancestor in ancestors {
                if ancestor
                    .get::<Option<serde_json::Value>, _>("policy")
                    .is_some()
                {
                    return Ok(false);
                }
                if ancestor.get::<String, _>("id") == entry_id.as_str() {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
    };
    match scope {
        PolicyScope::Root { .. } => Ok(matches!(old.source, PolicyScope::Root { .. })),
        PolicyScope::Entry { entry_id, .. } => {
            let ancestors=db::query("WITH RECURSIVE a AS (SELECT id,parent_id,policy,0 depth FROM entries WHERE root_id=$1 AND id=$2 UNION ALL SELECT e.id,e.parent_id,e.policy,a.depth+1 FROM entries e JOIN a ON e.id=a.parent_id WHERE e.root_id=$1 AND a.depth<4096) SELECT id,policy FROM a ORDER BY depth")
                .bind(root).bind(entry).fetch_all(db).await?;
            for ancestor in ancestors {
                if ancestor.get::<String, _>("id") == entry_id.as_str() {
                    return Ok(true);
                }
                if ancestor
                    .get::<Option<serde_json::Value>, _>("policy")
                    .is_some()
                {
                    return Ok(false);
                }
            }
            Ok(false)
        }
    }
}

impl Writer {
    pub async fn preview_change(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &PreviewChangeRequest,
    ) -> Result<ChangePreview> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        let row = root_row(&mut tx, root.as_str()).await?;
        require(
            row.get::<String, _>("revision") == request.expected_root_revision.as_str(),
            ErrorCode::RevisionConflict,
        )?;
        validate_change(&mut tx, root.as_str(), &request.change).await?;
        if matches!(request.change, PolicyChange::Move { .. }) {
            authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        }
        let authority = authority(&mut tx, actor, root.as_str()).await?;
        let preview = id();
        db::query("INSERT INTO change_previews(id,root_id,principal_id,request,root_revision,mutation_serial,authority) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(&preview).bind(root.as_str()).bind(&actor.principal).bind(json(request)?).bind(request.expected_root_revision.as_str()).bind(row.get::<i64,_>("mutation_serial")).bind(authority).execute(&mut *tx).await?;
        crate::namespace_moves::initialize(&mut tx, root.as_str(), &preview, &request.change)
            .await?;
        let result = inspect(&load(&mut tx, root.as_str(), &preview).await?)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn inspect_preview(
        &self,
        actor: &Actor,
        root: &RootId,
        preview: &PreviewId,
    ) -> Result<ChangePreview> {
        let mut tx = self.transaction().await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        let row = load(&mut tx, root.as_str(), preview.as_str()).await?;
        check_preview(&mut tx, actor, &row).await?;
        inspect(&row)
    }
    pub async fn advance_preview(
        &self,
        actor: &Actor,
        root: &RootId,
        preview: &PreviewId,
    ) -> Result<ChangePreview> {
        let mut tx = self.transaction().await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        let row = load(&mut tx, root.as_str(), preview.as_str()).await?;
        let request = check_preview(&mut tx, actor, &row).await?;
        if row.get::<String, _>("state") != "planning" {
            return inspect(&row);
        }
        if !row.get::<bool, _>("namespace_done") {
            crate::namespace_moves::advance(&mut tx, root.as_str(), preview.as_str()).await?;
            let result = inspect(&load(&mut tx, root.as_str(), preview.as_str()).await?)?;
            tx.commit().await?;
            return Ok(result);
        }
        let proposed = validate_change(&mut tx, root.as_str(), &request.change).await?;
        let cursor: Option<String> = row.get("cursor");
        let (mut last_entry, mut last_sequence, mut ordinal): (String, i64, u64) = cursor
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or((String::new(), 0, 0));
        let rows=db::query("SELECT id,entry_id,sequence,length FROM file_versions WHERE root_id=$1 AND NOT retired AND (entry_id,-sequence)>($2,$3) ORDER BY entry_id,-sequence LIMIT 64")
            .bind(root.as_str()).bind(&last_entry).bind(last_sequence).fetch_all(&mut *tx).await?;
        let mut retire_count = 0i64;
        let mut loss_count = 0i64;
        let mut bytes = 0u64;
        for version in &rows {
            let entry: String = version.get("entry_id");
            ordinal = if entry == last_entry { ordinal + 1 } else { 0 };
            last_entry = entry.clone();
            last_sequence = -version.get::<i64, _>("sequence");
            let old = effective(&mut tx, root.as_str(), Some(&entry)).await?;
            if !affected(&mut tx, root.as_str(), &entry, &request.change, &old).await? {
                continue;
            }
            let retire = ordinal >= proposed.keep_versions.get();
            let loses =
                !retire && protected(&old.policy, ordinal) && !protected(&proposed, ordinal);
            if retire || loses {
                db::query("INSERT INTO preview_versions(root_id,preview_id,version_id,retire,loses_protection) VALUES($1,$2,$3,$4,$5)")
                    .bind(root.as_str()).bind(preview.as_str()).bind(version.get::<String,_>("id")).bind(retire).bind(loses).execute(&mut *tx).await?;
                if retire {
                    retire_count += 1;
                    bytes = bytes
                        .saturating_add(version.get::<i64, _>("length") as u64)
                        .min(MAX_SAFE_INTEGER);
                } else {
                    loss_count += 1;
                }
            }
        }
        db::query("UPDATE change_previews SET state=$2,cursor=$3,processed=processed+$4,retire_count=retire_count+$5,loss_count=loss_count+$6,estimated_bytes=LEAST(9007199254740991,estimated_bytes::numeric+$7::bigint)::bigint WHERE id=$1")
            .bind(preview.as_str()).bind(if rows.len()<64 {"ready"}else{"planning"}).bind(serde_json::to_string(&(last_entry,last_sequence,ordinal))?).bind(rows.len() as i64).bind(retire_count).bind(loss_count).bind(bytes as i64).execute(&mut *tx).await?;
        let result = inspect(&load(&mut tx, root.as_str(), preview.as_str()).await?)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn preview_versions(
        &self,
        actor: &Actor,
        root: &RootId,
        preview: &PreviewId,
        request: &ListRequest,
    ) -> Result<PreviewVersionPage> {
        let mut tx = self.transaction().await?;
        lock_content_root(&mut tx, root.as_str()).await?;
        let row = load(&mut tx, root.as_str(), preview.as_str()).await?;
        check_preview(&mut tx, actor, &row).await?;
        require(
            row.get::<String, _>("state") == "ready",
            ErrorCode::InvalidRequest,
        )?;
        let binding = serde_json::json!({"root":root,"preview":preview,"limit":request.limit});
        let after: String = if let Some(cursor) = &request.cursor {
            crate::paging::consume(&mut tx, actor, "preview_versions", cursor, &binding).await?
        } else {
            String::new()
        };
        let rows=db::query("SELECT p.*,v.entry_id FROM preview_versions p JOIN file_versions v ON v.root_id=p.root_id AND v.id=p.version_id WHERE p.root_id=$1 AND p.preview_id=$2 AND p.version_id>$3 ORDER BY p.version_id LIMIT $4")
            .bind(root.as_str()).bind(preview.as_str()).bind(after).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let versions: Vec<PreviewVersion> = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|r| {
                Ok(PreviewVersion {
                    version_id: r.get::<String, _>("version_id").parse()?,
                    entry_id: r.get::<String, _>("entry_id").parse()?,
                    retire: r.get("retire"),
                    loses_protection: r.get("loses_protection"),
                })
            })
            .collect::<Result<_>>()?;
        let next_cursor = if rows.len() > request.limit.get() as usize {
            Some(
                crate::paging::save(
                    &mut tx,
                    actor,
                    "preview_versions",
                    binding,
                    json(&versions.last().unwrap().version_id)?,
                )
                .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        Ok(PreviewVersionPage {
            versions,
            next_cursor,
        })
    }
}
