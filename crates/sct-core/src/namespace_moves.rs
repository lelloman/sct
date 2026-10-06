use crate::{
    error::{Error, Result, id, require},
    policy::effective,
    roots::revision,
};
use db::PgConnection;
use sct_protocol::*;
use simple_server::database::postgres as db;

pub(crate) async fn validate(
    db: &mut PgConnection,
    root: &str,
    change: &PolicyChange,
) -> Result<(RetentionPolicy, i64)> {
    let enabled: bool = db::query_scalar("SELECT state='enabled' FROM roots WHERE id=$1")
        .bind(root)
        .fetch_one(&mut *db)
        .await?;
    require(enabled, ErrorCode::DestinationUnavailable)?;
    let PolicyChange::Move {
        entry_id,
        expected_revision,
        parent_id,
        expected_parent_revision,
        name,
    } = change
    else {
        return Err(Error(ErrorCode::InvalidRequest));
    };
    let source = db::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2")
        .bind(root)
        .bind(entry_id.as_str())
        .fetch_optional(&mut *db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    let destination = db::query("SELECT * FROM entries WHERE root_id=$1 AND id=$2")
        .bind(root)
        .bind(parent_id.as_str())
        .fetch_optional(&mut *db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    revision(&source, expected_revision)?;
    revision(&destination, expected_parent_revision)?;
    require(
        source.get::<Option<String>, _>("parent_id").is_some()
            && destination.get::<String, _>("kind") == "directory",
        ErrorCode::InvalidRequest,
    )?;
    require(
        !crate::namespace::hidden(db, entry_id.as_str()).await?
            && !crate::namespace::hidden(db, parent_id.as_str()).await?,
        ErrorCode::NotFound,
    )?;
    let source_policy = effective(db, root, Some(entry_id.as_str())).await?;
    let destination_policy = effective(db, root, Some(parent_id.as_str())).await?;
    require(
        source_policy.versioning == VersioningMode::IndependentFiles
            && destination_policy.versioning == VersioningMode::IndependentFiles,
        ErrorCode::InvalidRequest,
    )?;
    let ancestors=db::query("WITH RECURSIVE a AS (SELECT id,parent_id,name FROM entries WHERE root_id=$1 AND id=$2 UNION ALL SELECT e.id,e.parent_id,e.name FROM entries e JOIN a ON e.id=a.parent_id WHERE e.root_id=$1) SELECT id,parent_id,name FROM a")
        .bind(root).bind(parent_id.as_str()).fetch_all(&mut *db).await?;
    require(
        !ancestors
            .iter()
            .any(|r| r.get::<String, _>("id") == entry_id.as_str()),
        ErrorCode::InvalidRequest,
    )?;
    let prefix = ancestors
        .iter()
        .filter(|r| r.get::<Option<String>, _>("parent_id").is_some())
        .map(|r| r.get::<String, _>("name").chars().count() as i64 + 1)
        .sum::<i64>()
        + name.as_str().chars().count() as i64
        + 1;
    require(prefix <= 4096, ErrorCode::InvalidPath)?;
    let collision:bool=db::query_scalar("SELECT EXISTS(SELECT 1 FROM entries WHERE root_id=$1 AND parent_id=$2 AND name=$3 AND NOT hidden AND id<>$4)")
        .bind(root).bind(parent_id.as_str()).bind(name.as_str()).bind(entry_id.as_str()).fetch_one(db).await?;
    require(!collision, ErrorCode::RevisionConflict)?;
    Ok((destination_policy.policy, prefix))
}

pub(crate) async fn initialize(
    db: &mut PgConnection,
    root: &str,
    preview: &str,
    change: &PolicyChange,
) -> Result<()> {
    if let PolicyChange::Move { entry_id, .. } = change {
        let (_, length) = validate(db, root, change).await?;
        db::query("UPDATE change_previews SET namespace_done=false WHERE id=$1")
            .bind(preview)
            .execute(&mut *db)
            .await?;
        db::query("INSERT INTO preview_entry_work(root_id,preview_id,entry_id,path_length) VALUES($1,$2,$3,$4)")
            .bind(root).bind(preview).bind(entry_id.as_str()).bind(length as i32).execute(db).await?;
    }
    Ok(())
}
/// Validate paths and atomic boundaries one bounded child page at a time.
pub(crate) async fn advance(db: &mut PgConnection, root: &str, preview: &str) -> Result<()> {
    let work=db::query("SELECT * FROM preview_entry_work WHERE root_id=$1 AND preview_id=$2 ORDER BY entry_id LIMIT 1")
        .bind(root).bind(preview).fetch_optional(&mut *db).await?;
    if let Some(work) = work {
        let entry: String = work.get("entry_id");
        let length: i32 = work.get("path_length");
        let children=db::query("SELECT e.id,e.name,e.versioning,EXISTS(SELECT 1 FROM entries c WHERE c.root_id=e.root_id AND c.parent_id=e.id) children FROM entries e WHERE e.root_id=$1 AND e.parent_id=$2 AND e.id>$3 ORDER BY e.id LIMIT 128")
            .bind(root).bind(&entry).bind(work.get::<String,_>("after_child")).fetch_all(&mut *db).await?;
        for child in &children {
            require(
                child.get::<String, _>("versioning") == "independent_files",
                ErrorCode::InvalidRequest,
            )?;
            let child_length = length + child.get::<String, _>("name").chars().count() as i32 + 1;
            require(child_length <= 4096, ErrorCode::InvalidPath)?;
            if child.get::<bool, _>("children") {
                db::query("INSERT INTO preview_entry_work(root_id,preview_id,entry_id,path_length) VALUES($1,$2,$3,$4)")
                    .bind(root).bind(preview).bind(child.get::<String,_>("id")).bind(child_length).execute(&mut *db).await?;
            }
        }
        if children.len() == 128 {
            db::query("UPDATE preview_entry_work SET after_child=$4 WHERE root_id=$1 AND preview_id=$2 AND entry_id=$3")
                .bind(root).bind(preview).bind(entry).bind(children.last().unwrap().get::<String,_>("id")).execute(&mut *db).await?;
        } else {
            db::query(
                "DELETE FROM preview_entry_work WHERE root_id=$1 AND preview_id=$2 AND entry_id=$3",
            )
            .bind(root)
            .bind(preview)
            .bind(entry)
            .execute(&mut *db)
            .await?;
        }
    }
    db::query("UPDATE change_previews SET namespace_done=true WHERE root_id=$1 AND id=$2 AND NOT EXISTS(SELECT 1 FROM preview_entry_work WHERE root_id=$1 AND preview_id=$2)")
        .bind(root).bind(preview).execute(db).await?;
    Ok(())
}
pub(crate) async fn apply(db: &mut PgConnection, root: &str, change: &PolicyChange) -> Result<()> {
    let PolicyChange::Move {
        entry_id,
        parent_id,
        name,
        ..
    } = change
    else {
        return Err(Error(ErrorCode::InvalidRequest));
    };
    validate(db, root, change).await?;
    let source: String =
        db::query_scalar("SELECT parent_id FROM entries WHERE root_id=$1 AND id=$2")
            .bind(root)
            .bind(entry_id.as_str())
            .fetch_one(&mut *db)
            .await?;
    db::query("UPDATE entries SET parent_id=$3,name=$4,revision=$5 WHERE root_id=$1 AND id=$2")
        .bind(root)
        .bind(entry_id.as_str())
        .bind(parent_id.as_str())
        .bind(name.as_str())
        .bind(id())
        .execute(&mut *db)
        .await?;
    db::query("UPDATE entries SET revision=$4 WHERE root_id=$1 AND (id=$2 OR id=$3)")
        .bind(root)
        .bind(source)
        .bind(parent_id.as_str())
        .bind(id())
        .execute(db)
        .await?;
    Ok(())
}
