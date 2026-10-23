use serde::Serialize;

/// Errors surfaced to the UI. Every variant carries a message meant for the
/// user; `kind` lets the UI choose how to present it.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Audio(#[from] uta_audio::AudioError),
    #[error("{0}")]
    Store(#[from] uta_project::StoreError),
    #[error("{0}")]
    Edit(#[from] uta_project::EditError),
    #[error("no project is open")]
    NoProject,
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Busy(String),
    #[error("file error: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn kind(&self) -> &'static str {
        match self {
            CoreError::Audio(uta_audio::AudioError::Disconnected(_)) => "deviceLost",
            CoreError::Audio(uta_audio::AudioError::NoDevice(_)) => "noDevice",
            CoreError::Audio(_) => "audio",
            CoreError::Store(_) => "storage",
            CoreError::Edit(_) => "edit",
            CoreError::NoProject => "noProject",
            CoreError::NotFound(_) => "notFound",
            CoreError::Invalid(_) => "invalid",
            CoreError::Busy(_) => "busy",
            CoreError::Io(_) => "io",
            CoreError::Other(_) => "other",
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Invalid(e.to_string())
    }
}

/// Serializable error for IPC.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub kind: String,
    pub message: String,
}

impl From<&CoreError> for ErrorPayload {
    fn from(e: &CoreError) -> Self {
        Self {
            kind: e.kind().into(),
            message: e.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
