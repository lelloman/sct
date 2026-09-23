use crate::{
    error::{Error, Result, hash, id, require},
    lease::Writer,
};
use sct_protocol::{Capability, ErrorCode, PrincipalId};
use simple_server::auth::Access;
use sqlx::{PgConnection, Row};

/// Constructed only by credential authentication; authority is rechecked per transaction.
#[derive(Clone)]
pub struct Actor {
    pub(crate) principal: String,
    pub(crate) credential: String,
}
impl Actor {
    pub fn principal_id(&self) -> &str {
        &self.principal
    }
}
/// Secrets are returned once, intentionally have no Debug implementation.
pub struct IssuedCredential {
    pub id: String,
    pub secret: String,
    pub csrf: Option<String>,
}

pub(crate) async fn administrator(db: &mut PgConnection, actor: &Actor) -> Result<bool> {
    sqlx::query_scalar("SELECT p.administrator FROM principals p JOIN credentials c ON c.principal_id=p.id WHERE p.id=$1 AND c.id=$2 AND NOT p.disabled AND NOT c.revoked AND (c.expires_at IS NULL OR c.expires_at>clock_timestamp()) FOR SHARE OF p,c")
      .bind(&actor.principal).bind(&actor.credential).fetch_optional(db).await?
      .ok_or(Error(ErrorCode::Unauthorized))
}
pub(crate) async fn admin(db: &mut PgConnection, actor: &Actor) -> Result<()> {
    let administrator = administrator(db, actor).await?;
    Access::new(|administrator: &bool| Ok(*administrator))
        .with_check(|administrator, _| require(*administrator, ErrorCode::Forbidden))
        .evaluate(&administrator)?;
    Ok(())
}
pub(crate) async fn capabilities(
    db: &mut PgConnection,
    actor: &Actor,
    root: &str,
) -> Result<Vec<Capability>> {
    let is_admin = administrator(db, actor).await?;
    let owner: String = sqlx::query_scalar("SELECT owner_id FROM roots WHERE id=$1")
        .bind(root)
        .fetch_optional(&mut *db)
        .await?
        .ok_or(Error(ErrorCode::NotFound))?;
    if is_admin || owner == actor.principal {
        return Ok(all_capabilities());
    }
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT capabilities FROM grants WHERE root_id=$1 AND principal_id=$2")
            .bind(root)
            .bind(&actor.principal)
            .fetch_optional(db)
            .await?;
    value
        .map(serde_json::from_value)
        .transpose()?
        .ok_or(Error(ErrorCode::NotFound))
}
pub(crate) async fn authorize(
    db: &mut PgConnection,
    actor: &Actor,
    root: &str,
    capability: Capability,
) -> Result<()> {
    // Browser sessions manage accounts/policies/archives but never act as
    // program content writers, even when the principal is an administrator.
    if matches!(
        capability,
        Capability::Write | Capability::Delete | Capability::Purge
    ) {
        let kind: Option<String> =
            sqlx::query_scalar("SELECT kind FROM credentials WHERE id=$1 AND principal_id=$2")
                .bind(&actor.credential)
                .bind(&actor.principal)
                .fetch_optional(&mut *db)
                .await?;
        require(kind.as_deref() == Some("token"), ErrorCode::Forbidden)?;
    }
    let granted = capabilities(db, actor, root).await?;
    let permitted = granted.contains(&capability);
    Access::new(|permitted: &bool| Ok(*permitted))
        .with_check(|permitted, _| require(*permitted, ErrorCode::Forbidden))
        .evaluate(&permitted)?;
    Ok(())
}
pub fn all_capabilities() -> Vec<Capability> {
    vec![
        Capability::ListRead,
        Capability::Write,
        Capability::ManagePolicy,
        Capability::Delete,
        Capability::Purge,
        Capability::ManageGrants,
        Capability::ManageRoot,
    ]
}

async fn issue(db: &mut PgConnection, principal: &str, session: bool) -> Result<IssuedCredential> {
    let credential = IssuedCredential {
        id: id(),
        secret: format!("{}{}", id(), id()),
        csrf: session.then(|| format!("{}{}", id(), id())),
    };
    sqlx::query("INSERT INTO credentials(id,principal_id,secret_hash,kind,csrf_hash,expires_at) VALUES($1,$2,$3,$4,$5,CASE WHEN $4='session' THEN clock_timestamp()+interval '12 hours' ELSE NULL END)")
      .bind(&credential.id).bind(principal).bind(hash(&credential.secret)).bind(if session {"session"} else {"token"}).bind(credential.csrf.as_deref().map(hash)).execute(db).await?;
    Ok(credential)
}
impl Writer {
    /// Exchange an authenticated program credential for a short-lived browser
    /// session; the browser receives only secure session/CSRF cookies.
    pub async fn browser_session(&self, actor: &Actor) -> Result<IssuedCredential> {
        let mut tx = self.transaction().await?;
        administrator(&mut tx, actor).await?;
        let credential = issue(&mut tx, &actor.principal, true).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "session.created",
            serde_json::json!({"credential_id":credential.id}),
        )
        .await?;
        tx.commit().await?;
        Ok(credential)
    }
    /// Local process administration only. Requires the exclusive writer lease;
    /// run with the server stopped. Replaces all credentials of the local admin.
    pub async fn recover_administrator(&self) -> Result<IssuedCredential> {
        let mut tx = self.transaction().await?;
        sqlx::query("INSERT INTO principals(id,administrator) VALUES('local_admin',true) ON CONFLICT(id) DO UPDATE SET administrator=true,disabled=false").execute(&mut *tx).await?;
        sqlx::query("UPDATE credentials SET revoked=true WHERE principal_id='local_admin'")
            .execute(&mut *tx)
            .await?;
        let credential = issue(&mut tx, "local_admin", false).await?;
        tx.commit().await?;
        Ok(credential)
    }
    pub async fn authenticate(
        &self,
        secret: &str,
        session: bool,
        csrf: Option<&str>,
    ) -> Result<Actor> {
        require(secret.len() == 64, ErrorCode::Unauthorized)?;
        let mut tx = self.control_transaction().await?;
        let row = sqlx::query("SELECT c.id,c.principal_id,c.csrf_hash FROM credentials c JOIN principals p ON p.id=c.principal_id WHERE c.secret_hash=$1 AND c.kind=$2 AND NOT c.revoked AND NOT p.disabled AND (c.expires_at IS NULL OR c.expires_at>clock_timestamp())")
          .bind(hash(secret)).bind(if session {"session"} else {"token"}).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::Unauthorized))?;
        if let Some(csrf) = csrf {
            require(
                session && row.get::<Option<String>, _>("csrf_hash") == Some(hash(csrf)),
                ErrorCode::Forbidden,
            )?;
        }
        Ok(Actor {
            principal: row.get("principal_id"),
            credential: row.get("id"),
        })
    }
    pub async fn create_principal(&self, actor: &Actor, principal: &PrincipalId) -> Result<()> {
        let mut tx = self.transaction().await?;
        admin(&mut tx, actor).await?;
        sqlx::query("INSERT INTO principals(id) VALUES($1)")
            .bind(principal.as_str())
            .execute(&mut *tx)
            .await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "principal.created",
            serde_json::json!({"principal_id":principal}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn issue_token(
        &self,
        actor: &Actor,
        principal: &PrincipalId,
    ) -> Result<IssuedCredential> {
        let mut tx = self.transaction().await?;
        let is_admin = administrator(&mut tx, actor).await?;
        require(
            is_admin || actor.principal == principal.as_str(),
            ErrorCode::Forbidden,
        )?;
        let enabled: bool =
            sqlx::query_scalar("SELECT NOT disabled FROM principals WHERE id=$1 FOR SHARE")
                .bind(principal.as_str())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error(ErrorCode::NotFound))?;
        require(enabled, ErrorCode::InvalidRequest)?;
        let result = issue(&mut tx, principal.as_str(), false).await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "credential.issued",
            serde_json::json!({"principal_id": principal,"credential_id":result.id}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn revoke_credential(&self, actor: &Actor, credential: &str) -> Result<()> {
        let mut tx = self.transaction().await?;
        let is_admin = administrator(&mut tx, actor).await?;
        let principal: String =
            sqlx::query_scalar("SELECT principal_id FROM credentials WHERE id=$1")
                .bind(credential)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error(ErrorCode::NotFound))?;
        require(
            is_admin || actor.principal == principal,
            ErrorCode::Forbidden,
        )?;
        sqlx::query("UPDATE credentials SET revoked=true WHERE id=$1")
            .bind(credential)
            .execute(&mut *tx)
            .await?;
        crate::management::audit(
            &mut tx,
            actor,
            None,
            "credential.revoked",
            serde_json::json!({"credential_id":credential,"principal_id":principal}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    /// Only call after cryptographic OIDC verification of issuer, audience, expiry,
    /// nonce and authorization-code/PKCE binding. Email is never an identity key.
    pub async fn oidc_session(&self, issuer: &str, subject: &str) -> Result<IssuedCredential> {
        require(
            !issuer.is_empty()
                && !subject.is_empty()
                && issuer.len() <= 2048
                && subject.len() <= 1024,
            ErrorCode::Unauthorized,
        )?;
        let mut tx = self.transaction().await?;
        let principal: String=sqlx::query_scalar("INSERT INTO principals(id,issuer,subject) VALUES($1,$2,$3) ON CONFLICT(issuer,subject) DO UPDATE SET subject=EXCLUDED.subject RETURNING id")
          .bind(id()).bind(issuer).bind(subject).fetch_one(&mut *tx).await?;
        let disabled: bool = sqlx::query_scalar("SELECT disabled FROM principals WHERE id=$1")
            .bind(&principal)
            .fetch_one(&mut *tx)
            .await?;
        require(!disabled, ErrorCode::Unauthorized)?;
        let credential = issue(&mut tx, &principal, true).await?;
        tx.commit().await?;
        Ok(credential)
    }
    pub async fn save_oidc_flow(
        &self,
        state: &str,
        browser: &str,
        nonce: &str,
        verifier: &str,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        sqlx::query("DELETE FROM oidc_flows WHERE expires_at<=clock_timestamp()")
            .execute(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM oidc_flows")
            .fetch_one(&mut *tx)
            .await?;
        require(count < 10000, ErrorCode::RateLimited)?;
        sqlx::query("INSERT INTO oidc_flows(state_hash,browser_hash,nonce,pkce_verifier) VALUES($1,$2,$3,$4)")
          .bind(hash(state)).bind(hash(browser)).bind(nonce).bind(verifier).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn consume_oidc_flow(&self, state: &str, browser: &str) -> Result<(String, String)> {
        let mut tx = self.transaction().await?;
        let row=sqlx::query("DELETE FROM oidc_flows WHERE state_hash=$1 AND browser_hash=$2 AND expires_at>clock_timestamp() RETURNING nonce,pkce_verifier")
          .bind(hash(state)).bind(hash(browser)).fetch_optional(&mut *tx).await?.ok_or(Error(ErrorCode::Unauthorized))?;
        tx.commit().await?;
        Ok((row.get("nonce"), row.get("pkce_verifier")))
    }
}

impl Writer {
    pub async fn revoke_session(&self, actor: &Actor, secret: &str) -> Result<()> {
        let mut tx = self.transaction().await?;
        administrator(&mut tx, actor).await?;
        sqlx::query("UPDATE credentials SET revoked=true WHERE principal_id=$1 AND kind='session' AND secret_hash=$2").bind(&actor.principal).bind(hash(secret)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
