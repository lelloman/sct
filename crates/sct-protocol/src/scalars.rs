use crate::{MAX_SAFE_INTEGER, ValidationError};
use serde::{Deserialize, Serialize};

macro_rules! token {
    ($name:ident, $pattern:literal, $max:expr, $ts:literal, $check:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        #[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
        #[cfg_attr(feature = "codegen", schemars(extend("pattern" = $pattern.replace('$', "$(?![\\s\\S])"), "minLength" = 1, "maxLength" = $max)))]
        #[cfg_attr(feature = "codegen", ts(type = $ts))]
        pub struct $name(String);
        impl $name {
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl TryFrom<String> for $name {
            type Error = ValidationError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty() || value.chars().count() > $max || !($check)(&value) {
                    return Err(ValidationError(concat!("invalid ", stringify!($name))));
                }
                Ok(Self(value))
            }
        }
        impl std::str::FromStr for $name {
            type Err = ValidationError;
            fn from_str(value: &str) -> Result<Self, Self::Err> { value.to_owned().try_into() }
        }
        impl From<$name> for String { fn from(value: $name) -> String { value.0 } }
    };
}
fn opaque(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
token!(
    RequestId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"RequestId\" }",
    opaque
);
token!(
    PrincipalId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"PrincipalId\" }",
    opaque
);
token!(
    CredentialId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"CredentialId\" }",
    opaque
);
token!(
    GrantId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"GrantId\" }",
    opaque
);
token!(
    RootId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"RootId\" }",
    opaque
);
token!(
    EntryId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"EntryId\" }",
    opaque
);
token!(
    VersionId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"VersionId\" }",
    opaque
);
token!(
    BackendId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"BackendId\" }",
    opaque
);
token!(
    PoolId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"PoolId\" }",
    opaque
);
token!(
    AllocationId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"AllocationId\" }",
    opaque
);
token!(
    UploadId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"UploadId\" }",
    opaque
);
token!(
    OperationId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"OperationId\" }",
    opaque
);
token!(
    PolicyId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"PolicyId\" }",
    opaque
);
token!(
    PreviewId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"PreviewId\" }",
    opaque
);
token!(
    CheckpointId,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"CheckpointId\" }",
    opaque
);
token!(
    Revision,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"Revision\" }",
    opaque
);
token!(
    IdempotencyKey,
    "^[A-Za-z0-9_-]+$",
    128,
    "string & { readonly __brand: \"IdempotencyKey\" }",
    opaque
);
token!(
    CursorToken,
    "^[A-Za-z0-9_-]+$",
    2048,
    "string & { readonly __brand: \"CursorToken\" }",
    opaque
);
token!(
    ManifestId,
    "^[a-f0-9]{64}$",
    64,
    "string & { readonly __brand: \"ManifestId\" }",
    |v: &str| v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
);
token!(
    ChunkDigest,
    "^[a-f0-9]{64}$",
    64,
    "string & { readonly __brand: \"ChunkDigest\" }",
    |v: &str| v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
);
fn valid_name(v: &str) -> bool {
    v != "." && v != ".." && !v.contains(['/', '\0'])
}
token!(
    EntryName,
    r"^(?!\.{1,2}$)[^/\u0000]+$",
    255,
    "string & { readonly __brand: \"EntryName\" }",
    valid_name
);
fn valid_path(v: &str) -> bool {
    v == "/"
        || (v.starts_with('/')
            && v[1..]
                .split('/')
                .all(|part| !part.is_empty() && part.chars().count() <= 255 && valid_name(part)))
}
token!(
    RootPath,
    r"^(?:/|/(?!(?:\.{1,2})(?:/|$))[^/\u0000]{1,255}(?:/(?!(?:\.{1,2})(?:/|$))[^/\u0000]{1,255})*)$",
    4096,
    "string & { readonly __brand: \"RootPath\" }",
    valid_path
);

macro_rules! integer {
    ($name:ident, $min:expr, $max:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "u64", into = "u64")]
        #[cfg_attr(feature = "codegen", derive(schemars::JsonSchema, ts_rs::TS))]
        #[cfg_attr(feature = "codegen", schemars(extend("minimum" = $min, "maximum" = $max)))]
        #[cfg_attr(feature = "codegen", ts(type = "number"))]
        pub struct $name(u64);
        impl $name {
            pub fn get(self) -> u64 {
                self.0
            }
        }
        impl TryFrom<u64> for $name {
            type Error = ValidationError;
            fn try_from(value: u64) -> Result<Self, Self::Error> {
                if !($min..=$max).contains(&value) {
                    return Err(ValidationError(concat!("invalid ", stringify!($name))));
                }
                Ok(Self(value))
            }
        }
        impl From<$name> for u64 {
            fn from(value: $name) -> u64 {
                value.0
            }
        }
    };
}
integer!(ByteCount, 0, MAX_SAFE_INTEGER);
integer!(CommitSequence, 0, MAX_SAFE_INTEGER);
integer!(TimestampMs, 0, MAX_SAFE_INTEGER);
integer!(PageSize, 1, 1000);
integer!(VersionCount, 1, 1_000_000);
integer!(Milliseconds, 1, 86_400_000);
integer!(BytesPerSecond, 1, MAX_SAFE_INTEGER);
