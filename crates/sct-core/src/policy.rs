use crate::{
    auth::{Actor, admin, authorize},
    error::{Result, id, require},
    lease::Writer,
    roots::{json, lock_root, parse, root_row},
};
use db::PgConnection;
use sct_protocol::*;
use simple_server::database::postgres as db;

pub(crate) async fn effective(
    db: &mut PgConnection,
    root: &str,
    entry: Option<&str>,
) -> Result<EffectivePolicy> {
    let root_row = root_row(db, root).await?;
    let mut result = EffectivePolicy {
        id: root_row.get::<String, _>("policy_id").parse()?,
        revision: root_row.get::<String, _>("policy_revision").parse()?,
        source: PolicyScope::Root {
            root_id: root.parse()?,
        },
        inherited: entry.is_some(),
        policy: parse(&root_row, "default_policy")?,
        versioning: VersioningMode::IndependentFiles,
        transitioning: false,
    };
    if let Some(entry) = entry {
        let rows=db::query("WITH RECURSIVE ancestors AS (SELECT e.*,0 depth FROM entries e WHERE id=$1 AND root_id=$2 UNION ALL SELECT p.*,a.depth+1 FROM entries p JOIN ancestors a ON p.id=a.parent_id AND p.root_id=a.root_id WHERE a.depth<4096) SELECT * FROM ancestors ORDER BY depth")
          .bind(entry).bind(root).fetch_all(&mut *db).await?;
        require(!rows.is_empty(), ErrorCode::NotFound)?;
        if let Some(row) = rows
            .iter()
            .find(|r| r.get::<Option<serde_json::Value>, _>("policy").is_some())
        {
            let source: String = row.get("id");
            result.id = row.get::<String, _>("policy_id").parse()?;
            result.revision = row.get::<String, _>("policy_revision").parse()?;
            result.inherited = source != entry;
            result.source = PolicyScope::Entry {
                root_id: root.parse()?,
                entry_id: source.parse()?,
            };
            result.policy = parse(row, "policy")?;
        }
        if rows
            .iter()
            .any(|r| r.get::<String, _>("versioning") == "atomic_directory")
        {
            result.versioning = VersioningMode::AtomicDirectory;
        }
    }
    result.transitioning=db::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE root_id=$1 AND kind='policy_transition' AND state IN ('active','recovering','cleaning_up'))").bind(root).fetch_one(db).await?;
    Ok(result)
}
pub(crate) fn scope_parts(scope: &PolicyScope) -> (&RootId, Option<&EntryId>) {
    match scope {
        PolicyScope::Root { root_id } => (root_id, None),
        PolicyScope::Entry { root_id, entry_id } => (root_id, Some(entry_id)),
    }
}
/// Ordinal is based only on descending server commit sequence, never client time.
/// Hidden content uses the same rule. Tombstones are not content versions.
pub fn protected(policy: &RetentionPolicy, ordinal: u64) -> bool {
    ordinal == 0 || (policy.mode == RetentionMode::Strict && ordinal < policy.keep_versions.get())
}
impl Writer {
    pub async fn effective_policy(
        &self,
        actor: &Actor,
        scope: &PolicyScope,
    ) -> Result<EffectivePolicy> {
        let (root, entry) = scope_parts(scope);
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        effective(&mut tx, root.as_str(), entry.map(EntryId::as_str)).await
    }
    /// M1 supports metadata-only policy changes; content-bearing scopes require M3 preview/apply.
    pub async fn set_policy(
        &self,
        actor: &Actor,
        request: &SetPolicyRequest,
    ) -> Result<EffectivePolicy> {
        let (root, entry) = scope_parts(&request.scope);
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        lock_root(&mut tx, root.as_str()).await?;
        let old = effective(&mut tx, root.as_str(), entry.map(EntryId::as_str)).await?;
        require(
            old.revision == request.expected_revision,
            ErrorCode::RevisionConflict,
        )?;
        let root_data = root_row(&mut tx, root.as_str()).await?;
        require(
            !root_data.get::<bool, _>("populated"),
            ErrorCode::UnsupportedOperation,
        )?;
        let versions = request.policy.keep_versions.get() as i64;
        require(
            versions >= root_data.get::<i64, _>("min_versions")
                && versions <= root_data.get::<i64, _>("max_versions")
                && (request.policy.mode == RetentionMode::Strict
                    || root_data.get::<bool, _>("allow_relaxed")),
            ErrorCode::Forbidden,
        )?;
        if let Some(entry) = entry {
            let row = db::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2")
                .bind(root.as_str())
                .bind(entry.as_str())
                .fetch_one(&mut *tx)
                .await?;
            require(
                row.get::<String, _>("revision") == request.expected_scope_revision.as_str(),
                ErrorCode::RevisionConflict,
            )?;
            if let Some(parent) = row.get::<Option<String>, _>("parent_id") {
                require(
                    effective(&mut tx, root.as_str(), Some(&parent))
                        .await?
                        .versioning
                        != VersioningMode::AtomicDirectory,
                    ErrorCode::InvalidRequest,
                )?;
            }
            let atomic = request.versioning == VersioningMode::AtomicDirectory;
            require(
                !atomic || row.get::<String, _>("kind") == "directory",
                ErrorCode::InvalidRequest,
            )?;
            let changing_mode =
                (row.get::<String, _>("versioning") == "atomic_directory") != atomic;
            if changing_mode {
                let populated:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE parent_id=$1) OR EXISTS(SELECT 1 FROM entries WHERE id=$1 AND current_version_id IS NOT NULL)").bind(entry.as_str()).fetch_one(&mut *tx).await?;
                require(!populated, ErrorCode::InvalidRequest)?;
            }
            db::query("UPDATE entries SET policy_id=COALESCE(policy_id,$2),policy_revision=$3,policy=$4,versioning=$5,revision=$6 WHERE id=$1")
              .bind(entry.as_str()).bind(id()).bind(id()).bind(json(&request.policy)?).bind(json(&request.versioning)?.as_str().unwrap()).bind(id()).execute(&mut *tx).await?;
        } else {
            require(
                request.versioning == VersioningMode::IndependentFiles,
                ErrorCode::InvalidRequest,
            )?;
            require(
                root_data.get::<String, _>("revision") == request.expected_scope_revision.as_str(),
                ErrorCode::RevisionConflict,
            )?;
            db::query(
                "UPDATE roots SET default_policy=$2,policy_revision=$3,revision=$4 WHERE id=$1",
            )
            .bind(root.as_str())
            .bind(json(&request.policy)?)
            .bind(id())
            .bind(id())
            .execute(&mut *tx)
            .await?;
        }
        let result = effective(&mut tx, root.as_str(), entry.map(EntryId::as_str)).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn set_policy_constraints(
        &self,
        actor: &Actor,
        root: &RootId,
        expected: &Revision,
        min: VersionCount,
        max: VersionCount,
        allow_relaxed: bool,
    ) -> Result<()> {
        require(min.get() <= max.get(), ErrorCode::InvalidRequest)?;
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        lock_root(&mut tx, root.as_str()).await?;
        let row = root_row(&mut tx, root.as_str()).await?;
        crate::roots::revision(&row, expected)?;
        let mut policies: Vec<serde_json::Value> =
            db::query_scalar("SELECT policy FROM entries WHERE root_id=$1 AND policy IS NOT NULL")
                .bind(root.as_str())
                .fetch_all(&mut *tx)
                .await?;
        policies.push(row.get("default_policy"));
        for value in policies {
            let p: RetentionPolicy = serde_json::from_value(value)?;
            require(
                p.keep_versions.get() >= min.get()
                    && p.keep_versions.get() <= max.get()
                    && (allow_relaxed || p.mode == RetentionMode::Strict),
                ErrorCode::InvalidRequest,
            )?;
        }
        db::query("UPDATE roots SET min_versions=$2,max_versions=$3,allow_relaxed=$4,revision=$5 WHERE id=$1").bind(root.as_str()).bind(min.get() as i64).bind(max.get() as i64).bind(allow_relaxed).bind(id()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_and_commit_order_protection() {
        for keep in [1, 3] {
            let strict = RetentionPolicy {
                mode: RetentionMode::Strict,
                keep_versions: keep.try_into().unwrap(),
            };
            let relaxed = RetentionPolicy {
                mode: RetentionMode::Relaxed,
                ..strict.clone()
            };
            for ordinal in 0..5 {
                assert_eq!(protected(&strict, ordinal), ordinal < keep);
                assert_eq!(protected(&relaxed, ordinal), ordinal == 0);
            }
        }
    }
}
