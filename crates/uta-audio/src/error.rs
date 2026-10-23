use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio device error: {0}")]
    Device(String),
    #[error("no audio device available: {0}")]
    NoDevice(String),
    #[error("the audio device was disconnected: {0}")]
    Disconnected(String),
    #[error("unsupported audio configuration: {0}")]
    Unsupported(String),
    #[error("could not read audio file {path}: {reason}")]
    Read { path: PathBuf, reason: String },
    #[error("could not write audio file {path}: {reason}")]
    Write { path: PathBuf, reason: String },
    #[error("the audio engine is not running")]
    NotRunning,
    #[error("the engine is busy: {0}")]
    Busy(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl AudioError {
    pub fn read(path: impl Into<PathBuf>, reason: impl ToString) -> Self {
        AudioError::Read {
            path: path.into(),
            reason: reason.to_string(),
        }
    }
    pub fn write(path: impl Into<PathBuf>, reason: impl ToString) -> Self {
        AudioError::Write {
            path: path.into(),
            reason: reason.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, AudioError>;
