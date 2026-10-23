//! Error types.

use std::path::PathBuf;

use crate::model::Id;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("track {0} not found")]
    TrackNotFound(Id),
    #[error("clip {0} not found")]
    ClipNotFound(Id),
    #[error("asset {0} not found")]
    AssetNotFound(Id),
    #[error("effect {0} not found")]
    EffectNotFound(Id),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("file error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("project document is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project format version {found} is newer than this app supports ({supported}); update UTA Vocal Lab")]
    TooNew { found: u32, supported: u32 },
    #[error("no readable project revision found in {0}")]
    NoValidRevision(PathBuf),
    #[error("not a UTA project folder: {0}")]
    NotAProject(PathBuf),
    #[error("a project already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("unsafe asset path rejected: {0}")]
    UnsafePath(String),
    #[error("{0}")]
    Other(String),
}

impl StoreError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        StoreError::Io {
            path: path.into(),
            source,
        }
    }
}
