//! Project model, editing, undo/redo, persistence and the app-wide library.
//!
//! * [`model`] — the versioned project document (tracks, clips, takes,
//!   effect chains, plugin references, assets).
//! * [`edit`] — validated editing operations.
//! * [`history`] — bounded snapshot undo/redo.
//! * [`store`] — SQLite-backed project folders with revisions, autosave,
//!   recovery, asset validation and backups.
//! * [`migrate`] — document format migrations.
//! * [`library`] — the app-wide database (history, settings, presets…).

pub mod edit;
pub mod error;
pub mod history;
pub mod library;
pub mod migrate;
pub mod model;
pub mod paths;
pub mod store;

pub use error::{EditError, StoreError};
pub use history::History;
pub use library::Library;
pub use model::*;
pub use store::{AssetCheck, AssetStatus, LoadedProject, ProjectStore, RevisionInfo, RevisionKind};
