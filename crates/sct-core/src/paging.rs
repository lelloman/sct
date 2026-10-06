use crate::{
    auth::Actor,
    error::{Error, Result, hash, id, require},
};
use db::PgConnection;
use sct_protocol::{CursorToken, ErrorCode};
use serde_json::Value;
use simple_server::database::postgres as db;

pub(crate) async fn consume<T: serde::de::DeserializeOwned>(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    token: &CursorToken,
    binding: &Value,
) -> Result<T> {
    let row=db::query("SELECT binding,position FROM cursors WHERE token_hash=$1 AND principal_id=$2 AND scope=$3 AND expires_at>clock_timestamp()")
      .bind(hash(token.as_str())).bind(&actor.principal).bind(scope).fetch_optional(db).await?.ok_or(Error(ErrorCode::CursorRefreshRequired))?;
    require(
        row.get::<Value, _>("binding") == *binding,
        ErrorCode::CursorRefreshRequired,
    )?;
    Ok(serde_json::from_value(row.get("position"))?)
}
pub(crate) async fn save(
    db: &mut PgConnection,
    actor: &Actor,
    scope: &str,
    binding: Value,
    position: Value,
) -> Result<CursorToken> {
    // Random server-held tokens bind every query property and survive restarts.
    // Expired tokens are pruned opportunistically; active tokens have a per-user cap.
    db::query("DELETE FROM cursors WHERE expires_at<=clock_timestamp()")
        .execute(&mut *db)
        .await?;
    let count: i64 = db::query_scalar("SELECT count(*) FROM cursors WHERE principal_id=$1")
        .bind(&actor.principal)
        .fetch_one(&mut *db)
        .await?;
    require(count < 10000, ErrorCode::RateLimited)?;
    let token = format!("{}{}", id(), id());
    db::query("INSERT INTO cursors(token_hash,principal_id,scope,binding,position) VALUES($1,$2,$3,$4,$5)").bind(hash(&token)).bind(&actor.principal).bind(scope).bind(binding).bind(position).execute(db).await?;
    Ok(token.parse()?)
}
