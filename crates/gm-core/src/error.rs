use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a ground-model file: {0}")]
    NotARepository(String),

    #[error("ground-model file was written by schema {found}, this build understands {supported}")]
    SchemaVersion { found: String, supported: String },

    #[error("{0} not found: {1}")]
    NotFound(&'static str, String),

    #[error("object {0} is missing from the store")]
    MissingObject(String),

    #[error("object {hash} is corrupt: content hashes to {actual}")]
    CorruptObject { hash: String, actual: String },

    #[error("cannot canonicalise a non-finite number")]
    NonFiniteNumber,

    #[error("{0}")]
    Invalid(String),

    #[error("working tree has uncommitted changes; commit or discard them first")]
    DirtyWorkingTree,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}
