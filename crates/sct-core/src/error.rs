use sct_protocol::ErrorCode;

#[derive(Debug)]
pub struct Error(pub ErrorCode);
pub type Result<T> = std::result::Result<T, Error>;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        // Never expose database text, statements, connection strings or constraint values.
        let code = error.as_database_error().and_then(|e| e.code());
        Self(match code.as_deref() {
            Some("23505" | "40001" | "40P01") => ErrorCode::RevisionConflict,
            Some("23503" | "23514") => ErrorCode::InvalidRequest,
            _ => ErrorCode::InternalError,
        })
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self(ErrorCode::InvalidRequest)
    }
}
impl From<sct_protocol::ValidationError> for Error {
    fn from(_: sct_protocol::ValidationError) -> Self {
        Self(ErrorCode::InvalidRequest)
    }
}
pub(crate) fn require(value: bool, code: ErrorCode) -> Result<()> {
    if value { Ok(()) } else { Err(Error(code)) }
}
pub(crate) fn id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub(crate) fn hash(secret: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}
