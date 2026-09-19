use crate::{
    auth::{Actor, authorize},
    error::{Error, Result, id, require},
    lease::Writer,
    policy::effective,
    roots::{json, lock_root, remember, replay, revision},
};
use sct_protocol::*;
use serde_json::json;
use sqlx::{PgConnection, Row, postgres::PgRow};

pub(crate) async fn row(db: &mut PgConnection, entry: &str) -> Result<PgRow> {
    sqlx::query("SELECT * FROM entries WHERE id=$1")
        .bind(entry)
        .fetch_optional(db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))
}
pub(crate) fn inspect(row: &PgRow) -> Result<EntryInspection> {
    Ok(EntryInspection {
        id: row.get::<String, _>("id").parse()?,
        root_id: row.get::<String, _>("root_id").parse()?,
        parent_id: row
            .get::<Option<String>, _>("parent_id")
            .map(|v| v.parse())
            .transpose()?,
        name: row.get::<String, _>("name").parse()?,
        kind: serde_json::from_value(json!(row.get::<String, _>("kind")))?,
        revision: row.get::<String, _>("revision").parse()?,
        hidden: row.get("hidden"),
        current_version_id: row
            .get::<Option<String>, _>("current_version_id")
            .map(|v| v.parse())
            .transpose()?,
    })
}
pub(crate) async fn hidden(db: &mut PgConnection, entry: &str) -> Result<bool> {
    Ok(sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,hidden FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.hidden FROM entries e JOIN a ON e.id=a.parent_id) SELECT COALESCE(bool_or(hidden),true) FROM a")
        .bind(entry).fetch_one(db).await?)
}
async fn mutable_directory(db: &mut PgConnection, root: &str, entry: &str) -> Result<PgRow> {
    let row = row(db, entry).await?;
    require(
        row.get::<String, _>("root_id") == root
            && row.get::<String, _>("kind") == "directory"
            && !hidden(db, entry).await?,
        ErrorCode::InvalidRequest,
    )?;
    require(
        effective(db, root, Some(entry)).await?.versioning != VersioningMode::AtomicDirectory,
        ErrorCode::InvalidRequest,
    )?;
    Ok(row)
}
async fn path_length(db: &mut PgConnection, entry: &str) -> Result<i64> {
    Ok(sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id,name FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id,e.name FROM entries e JOIN a ON e.id=a.parent_id) SELECT COALESCE(sum(char_length(name)+1) FILTER(WHERE parent_id IS NOT NULL),0)::bigint FROM a").bind(entry).fetch_one(db).await?)
}
impl Writer {
    pub async fn inspect_entry(&self, actor: &Actor, entry: &EntryId) -> Result<EntryInspection> {
        let mut tx = self.transaction().await?;
        let initial = row(&mut tx, entry.as_str()).await?;
        let root: String = initial.get("root_id");
        sqlx::query("SELECT id FROM roots WHERE id=$1 FOR SHARE")
            .bind(&root)
            .execute(&mut *tx)
            .await?;
        let entry = row(&mut tx, entry.as_str()).await?;
        authorize(
            &mut tx,
            actor,
            &entry.get::<String, _>("root_id"),
            Capability::ListRead,
        )
        .await?;
        let mut result = inspect(&entry)?;
        result.hidden = hidden(&mut tx, result.id.as_str()).await?;
        Ok(result)
    }
    pub async fn resolve_path(
        &self,
        actor: &Actor,
        root: &RootId,
        path: &RootPath,
    ) -> Result<EntryInspection> {
        let mut tx = self.transaction().await?;
        // A shared root lock gives a coherent path walk while short namespace writes commit.
        sqlx::query("SELECT id FROM roots WHERE id=$1 FOR SHARE")
            .bind(root.as_str())
            .execute(&mut *tx)
            .await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let mut entry = sqlx::query("SELECT * FROM entries WHERE root_id=$1 AND parent_id IS NULL")
            .bind(root.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let components: Vec<String> = path
            .as_str()
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let mut components = components.into_iter();
        while let Some(component) = components.next() {
            if entry.get::<String, _>("versioning") == "atomic_directory" {
                let version = entry
                    .get::<Option<String>, _>("current_version_id")
                    .ok_or(Error(ErrorCode::NotFound))?;
                let mut manifest =
                    crate::tree_catalog::version_manifest(&mut tx, root.as_str(), &version).await?;
                let mut parent: EntryId = entry.get::<String, _>("id").parse()?;
                let mut components = std::iter::once(component).chain(components).peekable();
                while let Some(component) = components.next() {
                    let child = crate::tree_catalog::child(
                        &mut tx,
                        root.as_str(),
                        &manifest,
                        &component.parse()?,
                    )
                    .await?;
                    if components.peek().is_none() {
                        return Ok(EntryInspection {
                            id: child.id,
                            root_id: root.clone(),
                            parent_id: Some(parent),
                            name: child.name,
                            kind: child.kind,
                            revision: version.parse()?,
                            hidden: false,
                            current_version_id: None,
                        });
                    }
                    require(child.kind == EntryKind::Directory, ErrorCode::NotFound)?;
                    parent = child.id;
                    manifest = child.manifest.as_str().to_owned();
                }
                unreachable!();
            }
            entry=sqlx::query("SELECT * FROM entries WHERE root_id=$1 AND parent_id=$2 AND name=$3 AND NOT hidden").bind(root.as_str()).bind(entry.get::<String,_>("id")).bind(component).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::NotFound))?;
        }
        inspect(&entry)
    }
    pub async fn create_directory(
        &self,
        actor: &Actor,
        root: &RootId,
        request: &CreateDirectoryRequest,
    ) -> Result<EntryInspection> {
        let mut tx = self.transaction().await?;
        authorize(&mut tx, actor, root.as_str(), Capability::Write).await?;
        let scope = format!("directory:{}", root.as_str());
        if let Some(result) = replay(
            &mut tx,
            actor,
            &scope,
            &request.idempotency_key,
            &json(request)?,
        )
        .await?
        {
            return Ok(result);
        }
        lock_root(&mut tx, root.as_str()).await?;
        require(
            crate::roots::root_row(&mut tx, root.as_str())
                .await?
                .get::<String, _>("state")
                == "enabled",
            ErrorCode::Forbidden,
        )?;
        let parent = mutable_directory(&mut tx, root.as_str(), request.parent_id.as_str()).await?;
        revision(&parent, &request.expected_parent_revision)?;
        require(
            path_length(&mut tx, request.parent_id.as_str()).await?
                + (request.name.as_str().chars().count() as i64)
                < 4096,
            ErrorCode::InvalidPath,
        )?;
        let atomic = request.versioning == VersioningMode::AtomicDirectory;
        if atomic {
            authorize(&mut tx, actor, root.as_str(), Capability::ManagePolicy).await?;
        }
        let inherited = effective(&mut tx, root.as_str(), Some(request.parent_id.as_str())).await?;
        let entry = id();
        let row=sqlx::query("INSERT INTO entries(id,root_id,parent_id,name,kind,revision,versioning,policy_id,policy_revision,policy) VALUES($1,$2,$3,$4,'directory',$5,$6,$7,$8,$9) RETURNING *")
          .bind(entry).bind(root.as_str()).bind(request.parent_id.as_str()).bind(request.name.as_str()).bind(id()).bind(json(&request.versioning)?.as_str().unwrap()).bind(atomic.then(id)).bind(atomic.then(id)).bind(if atomic{Some(json(&inherited.policy)?)}else{None}).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE entries SET revision=$2 WHERE id=$1")
            .bind(request.parent_id.as_str())
            .bind(id())
            .execute(&mut *tx)
            .await?;
        let result = inspect(&row)?;
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
    pub async fn list_entries(
        &self,
        actor: &Actor,
        root: &RootId,
        directory: &EntryId,
        request: &ListEntriesRequest,
    ) -> Result<EntryPage> {
        let mut tx = self.transaction().await?;
        sqlx::query("SELECT id FROM roots WHERE id=$1 FOR SHARE")
            .bind(root.as_str())
            .execute(&mut *tx)
            .await?;
        authorize(&mut tx, actor, root.as_str(), Capability::ListRead).await?;
        let dir = row(&mut tx, directory.as_str()).await?;
        require(
            dir.get::<String, _>("root_id") == root.as_str()
                && dir.get::<String, _>("kind") == "directory",
            ErrorCode::NotFound,
        )?;
        let revision: Revision = dir.get::<String, _>("revision").parse()?;
        let inherited_hidden = hidden(&mut tx, directory.as_str()).await?;
        require(
            request.include_hidden || !inherited_hidden,
            ErrorCode::NotFound,
        )?;
        let scope = format!("entries:{}:{}", root.as_str(), directory.as_str());
        let binding = json!({"revision":revision,"limit":request.limit,"include_hidden":request.include_hidden,"hidden":inherited_hidden});
        let after: (String, String) = if let Some(cursor) = &request.cursor {
            require(
                cursor.directory_id == *directory && cursor.directory_revision == revision,
                ErrorCode::CursorRefreshRequired,
            )?;
            crate::paging::consume(&mut tx, actor, &scope, &cursor.token, &binding).await?
        } else {
            (String::new(), String::new())
        };
        let rows=sqlx::query("SELECT * FROM entries WHERE root_id=$1 AND parent_id=$2 AND ($3 OR NOT hidden) AND (name,id)>($4 COLLATE \"C\",$5) ORDER BY name COLLATE \"C\",id LIMIT $6")
          .bind(root.as_str()).bind(directory.as_str()).bind(request.include_hidden).bind(after.0).bind(after.1).bind(request.limit.get() as i64+1).fetch_all(&mut *tx).await?;
        let more = rows.len() > request.limit.get() as usize;
        let items: Vec<_> = rows
            .iter()
            .take(request.limit.get() as usize)
            .map(|row| {
                let mut entry = inspect(row)?;
                entry.hidden |= inherited_hidden;
                Ok(entry)
            })
            .collect::<Result<_>>()?;
        let next_cursor = if more {
            let last = items.last().unwrap();
            Some(DirectoryCursor {
                token: crate::paging::save(
                    &mut tx,
                    actor,
                    &scope,
                    binding,
                    json!([last.name, last.id]),
                )
                .await?,
                directory_id: directory.clone(),
                directory_revision: revision.clone(),
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(EntryPage {
            directory_id: directory.clone(),
            directory_revision: revision,
            items,
            next_cursor,
        })
    }
    pub async fn rename_entry(
        &self,
        actor: &Actor,
        entry: &EntryId,
        request: &RenameEntryRequest,
    ) -> Result<EntryInspection> {
        let mut tx = self.transaction().await?;
        let initial = row(&mut tx, entry.as_str()).await?;
        let root: String = initial.get("root_id");
        authorize(&mut tx, actor, &root, Capability::Write).await?;
        lock_root(&mut tx, &root).await?;
        require(
            crate::roots::root_row(&mut tx, &root)
                .await?
                .get::<String, _>("state")
                == "enabled",
            ErrorCode::Forbidden,
        )?;
        let current = row(&mut tx, entry.as_str()).await?;
        revision(&current, &request.expected_revision)?;
        require(
            !hidden(&mut tx, entry.as_str()).await?,
            ErrorCode::InvalidRequest,
        )?;
        let source: String = current
            .get::<Option<String>, _>("parent_id")
            .ok_or(Error(ErrorCode::InvalidRequest))?;
        let src = mutable_directory(&mut tx, &root, &source).await?;
        revision(&src, &request.expected_source_parent_revision)?;
        let dst = mutable_directory(&mut tx, &root, request.destination_parent_id.as_str()).await?;
        revision(&dst, &request.expected_destination_parent_revision)?;
        require(
            effective(&mut tx, &root, Some(entry.as_str()))
                .await?
                .versioning
                != VersioningMode::AtomicDirectory,
            ErrorCode::InvalidRequest,
        )?;
        let source_policy = effective(&mut tx, &root, Some(&source)).await?;
        let destination_policy =
            effective(&mut tx, &root, Some(request.destination_parent_id.as_str())).await?;
        // Conservative foundation rule: even equal values across distinct policy scopes
        // require an explicit future policy-effect workflow.
        require(
            source_policy.id == destination_policy.id
                && source_policy.revision == destination_policy.revision,
            ErrorCode::InvalidRequest,
        )?;
        let cycle:bool=sqlx::query_scalar("WITH RECURSIVE a AS (SELECT id,parent_id FROM entries WHERE id=$1 UNION ALL SELECT e.id,e.parent_id FROM entries e JOIN a ON e.id=a.parent_id) SELECT EXISTS(SELECT 1 FROM a WHERE id=$2)")
          .bind(request.destination_parent_id.as_str()).bind(entry.as_str()).fetch_one(&mut *tx).await?;
        require(!cycle, ErrorCode::InvalidRequest)?;
        let subtree_max:i64=sqlx::query_scalar("WITH RECURSIVE d AS (SELECT id,0::bigint length FROM entries WHERE id=$1 UNION ALL SELECT e.id,d.length+char_length(e.name)+1 FROM entries e JOIN d ON e.parent_id=d.id) SELECT max(length) FROM d")
          .bind(entry.as_str()).fetch_one(&mut *tx).await?;
        require(
            path_length(&mut tx, request.destination_parent_id.as_str()).await?
                + request.name.as_str().chars().count() as i64
                + 1
                + subtree_max
                <= 4096,
            ErrorCode::InvalidPath,
        )?;
        let changed = sqlx::query(
            "UPDATE entries SET parent_id=$2,name=$3,revision=$4 WHERE id=$1 RETURNING *",
        )
        .bind(entry.as_str())
        .bind(request.destination_parent_id.as_str())
        .bind(request.name.as_str())
        .bind(id())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE entries SET revision=$3 WHERE id=$1 OR id=$2")
            .bind(source)
            .bind(request.destination_parent_id.as_str())
            .bind(id())
            .execute(&mut *tx)
            .await?;
        let result = inspect(&changed)?;
        tx.commit().await?;
        Ok(result)
    }
}
