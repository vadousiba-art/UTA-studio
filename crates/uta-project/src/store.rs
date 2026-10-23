//! Project persistence.
//!
//! A project is a folder `<Name>.utaproj/` containing:
//!
//! ```text
//! project.utadb   SQLite: metadata + every saved/autosaved document revision
//! audio/          WAV assets (recordings, imports, renders) — never rewritten
//! cache/          regenerable caches (waveform peaks, stretched previews)
//! ```
//!
//! Each save inserts a new revision row inside a SQLite transaction
//! (`synchronous=FULL`, rollback journal), so a crash or power loss during a
//! save leaves the previous revision intact. Revisions carry a checksum; on
//! open, unreadable revisions are skipped and the newest valid one is used.
//! Autosaves are separate revisions; when one is newer than the last manual
//! save, the caller is told so it can offer recovery.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;
use crate::migrate::migrate;
use crate::model::{Asset, Id, Project, FORMAT_VERSION};
use crate::paths::{resolve_asset_path, AUDIO_DIR, CACHE_DIR, PROJECT_DB};

type Result<T> = std::result::Result<T, StoreError>;

/// Revisions kept per kind when pruning.
const KEEP_SAVES: i64 = 30;
const KEEP_AUTOSAVES: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    Save,
    Autosave,
}

impl RevisionKind {
    fn as_str(self) -> &'static str {
        match self {
            RevisionKind::Save => "save",
            RevisionKind::Autosave => "autosave",
        }
    }
    fn parse(s: &str) -> Self {
        if s == "autosave" {
            RevisionKind::Autosave
        } else {
            RevisionKind::Save
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionInfo {
    pub id: i64,
    pub kind: RevisionKind,
    pub created_at: DateTime<Utc>,
    pub format_version: u32,
}

#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub project: Project,
    pub revision: RevisionInfo,
    /// Format version the document was stored in (if it was migrated).
    pub migrated_from: Option<u32>,
    /// Number of newer revisions that were unreadable and skipped.
    pub skipped_corrupt: usize,
    /// An autosave newer than the loaded manual save (offer recovery).
    pub newer_autosave: Option<RevisionInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum AssetStatus {
    Ok,
    Missing,
    Mismatch { reason: String },
    Unreadable { reason: String },
    UnsafePath,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetCheck {
    pub asset_id: Id,
    pub file: String,
    pub status: AssetStatus,
}

pub struct ProjectStore {
    dir: PathBuf,
    conn: Connection,
}

fn checksum(s: &str) -> String {
    // FNV-1a 64: detects torn/corrupted documents (not a security hash).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(2))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS revisions (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             kind TEXT NOT NULL,
             created_at TEXT NOT NULL,
             format_version INTEGER NOT NULL,
             checksum TEXT NOT NULL,
             doc TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS revisions_kind ON revisions(kind, id);",
    )?;
    Ok(conn)
}

impl ProjectStore {
    /// Create a new project folder and write the first revision.
    pub fn create(dir: &Path, project: &Project) -> Result<Self> {
        if dir.join(PROJECT_DB).exists() {
            return Err(StoreError::AlreadyExists(dir.to_path_buf()));
        }
        if dir.exists() {
            let non_empty = fs::read_dir(dir)
                .map_err(|e| StoreError::io(dir, e))?
                .next()
                .is_some();
            if non_empty {
                return Err(StoreError::AlreadyExists(dir.to_path_buf()));
            }
        }
        for sub in [AUDIO_DIR, CACHE_DIR] {
            let p = dir.join(sub);
            fs::create_dir_all(&p).map_err(|e| StoreError::io(&p, e))?;
        }
        let conn = open_db(&dir.join(PROJECT_DB))?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('project_id', ?1), ('app', 'UTA Vocal Lab')",
            params![project.id.to_string()],
        )?;
        let mut store = Self {
            dir: dir.to_path_buf(),
            conn,
        };
        store.save(project, RevisionKind::Save)?;
        Ok(store)
    }

    /// Open an existing project folder and load its newest valid revision.
    pub fn open(dir: &Path) -> Result<(Self, LoadedProject)> {
        let db = dir.join(PROJECT_DB);
        if !db.is_file() {
            return Err(StoreError::NotAProject(dir.to_path_buf()));
        }
        let conn = open_db(&db)?;
        for sub in [AUDIO_DIR, CACHE_DIR] {
            let p = dir.join(sub);
            fs::create_dir_all(&p).map_err(|e| StoreError::io(&p, e))?;
        }
        let store = Self {
            dir: dir.to_path_buf(),
            conn,
        };
        let loaded = store.load_latest()?;
        Ok((store, loaded))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.dir.join(AUDIO_DIR)
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.dir.join(CACHE_DIR)
    }

    /// Absolute, validated path of an asset file.
    pub fn asset_path(&self, asset: &Asset) -> Result<PathBuf> {
        resolve_asset_path(&self.dir, &asset.file)
    }

    /// Persist a revision. Returns the revision id.
    pub fn save(&mut self, project: &Project, kind: RevisionKind) -> Result<i64> {
        let mut doc = project.clone();
        doc.format_version = FORMAT_VERSION;
        let json = serde_json::to_string(&doc)?;
        let sum = checksum(&json);
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO revisions(kind, created_at, format_version, checksum, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![kind.as_str(), Utc::now().to_rfc3339(), FORMAT_VERSION, sum, json],
        )?;
        let id = tx.last_insert_rowid();
        if kind == RevisionKind::Save {
            tx.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('name', ?1)",
                params![project.name],
            )?;
        }
        tx.commit()?;
        self.prune()?;
        Ok(id)
    }

    fn prune(&self) -> Result<()> {
        for (kind, keep) in [("save", KEEP_SAVES), ("autosave", KEEP_AUTOSAVES)] {
            self.conn.execute(
                "DELETE FROM revisions WHERE kind = ?1 AND id NOT IN
                   (SELECT id FROM revisions WHERE kind = ?1 ORDER BY id DESC LIMIT ?2)",
                params![kind, keep],
            )?;
        }
        Ok(())
    }

    pub fn revisions(&self) -> Result<Vec<RevisionInfo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, created_at, format_version FROM revisions ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, u32>(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, created, fv) = row?;
            out.push(RevisionInfo {
                id,
                kind: RevisionKind::parse(&kind),
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                format_version: fv,
            });
        }
        Ok(out)
    }

    /// Load one revision (validated, migrated). Returns the project and the
    /// version it was migrated from (if any).
    pub fn load_revision(&self, id: i64) -> Result<(Project, Option<u32>)> {
        let row: Option<(String, String)> = self
            .conn
            .query_row("SELECT checksum, doc FROM revisions WHERE id = ?1", params![id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (sum, doc) = row.ok_or_else(|| StoreError::Other(format!("revision {id} not found")))?;
        if checksum(&doc) != sum {
            return Err(StoreError::Other(format!("revision {id} failed its checksum")));
        }
        let value: serde_json::Value = serde_json::from_str(&doc)?;
        let (value, from) = migrate(value)?;
        let project: Project = serde_json::from_value(value)?;
        Ok((project, if from != FORMAT_VERSION { Some(from) } else { None }))
    }

    fn load_latest(&self) -> Result<LoadedProject> {
        let revs = self.revisions()?;
        let mut skipped = 0;
        // Prefer the newest manual save; fall back to any valid revision.
        let saves: Vec<&RevisionInfo> = revs.iter().filter(|r| r.kind == RevisionKind::Save).collect();
        let others: Vec<&RevisionInfo> = revs.iter().filter(|r| r.kind != RevisionKind::Save).collect();
        for rev in saves.iter().chain(others.iter()) {
            match self.load_revision(rev.id) {
                Ok((project, migrated_from)) => {
                    let newer_autosave = revs
                        .iter()
                        .find(|r| r.kind == RevisionKind::Autosave && r.id > rev.id)
                        .cloned();
                    return Ok(LoadedProject {
                        project,
                        revision: (*rev).clone(),
                        migrated_from,
                        skipped_corrupt: skipped,
                        newer_autosave,
                    });
                }
                Err(e @ StoreError::TooNew { .. }) => return Err(e),
                Err(e) => {
                    tracing::warn!("skipping unreadable revision {}: {e}", rev.id);
                    skipped += 1;
                }
            }
        }
        Err(StoreError::NoValidRevision(self.dir.clone()))
    }

    /// Check every asset file referenced by the project.
    pub fn validate_assets(&self, project: &Project) -> Vec<AssetCheck> {
        project
            .assets
            .iter()
            .map(|a| AssetCheck {
                asset_id: a.id,
                file: a.file.clone(),
                status: self.check_asset(a, project.sample_rate),
            })
            .collect()
    }

    fn check_asset(&self, a: &Asset, project_rate: u32) -> AssetStatus {
        let path = match self.asset_path(a) {
            Ok(p) => p,
            Err(_) => return AssetStatus::UnsafePath,
        };
        if !path.is_file() {
            return AssetStatus::Missing;
        }
        match hound::WavReader::open(&path) {
            Ok(r) => {
                let spec = r.spec();
                let frames = r.duration() as u64;
                if spec.sample_rate != a.sample_rate || spec.sample_rate != project_rate {
                    AssetStatus::Mismatch {
                        reason: format!("sample rate {} Hz, expected {} Hz", spec.sample_rate, project_rate),
                    }
                } else if spec.channels != a.channels {
                    AssetStatus::Mismatch {
                        reason: format!("{} channels, expected {}", spec.channels, a.channels),
                    }
                } else if frames < a.frames {
                    AssetStatus::Mismatch {
                        reason: format!("file has {frames} frames, expected {}", a.frames),
                    }
                } else {
                    AssetStatus::Ok
                }
            }
            Err(e) => AssetStatus::Unreadable { reason: e.to_string() },
        }
    }

    /// Write a consistent copy of the project (database + audio) to `dest`,
    /// which must not exist yet.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(StoreError::AlreadyExists(dest.to_path_buf()));
        }
        let audio = dest.join(AUDIO_DIR);
        fs::create_dir_all(&audio).map_err(|e| StoreError::io(&audio, e))?;
        fs::create_dir_all(dest.join(CACHE_DIR)).map_err(|e| StoreError::io(dest, e))?;
        let db = dest.join(PROJECT_DB);
        self.conn
            .execute("VACUUM INTO ?1", params![db.to_string_lossy().to_string()])?;
        let src_audio = self.audio_dir();
        if src_audio.is_dir() {
            for entry in fs::read_dir(&src_audio).map_err(|e| StoreError::io(&src_audio, e))? {
                let entry = entry.map_err(|e| StoreError::io(&src_audio, e))?;
                if entry.path().is_file() {
                    let to = audio.join(entry.file_name());
                    fs::copy(entry.path(), &to).map_err(|e| StoreError::io(&to, e))?;
                }
            }
        }
        Ok(())
    }

    /// Delete an asset file that no clip references any more. Refuses if it is
    /// still referenced.
    pub fn delete_unused_asset(&self, project: &mut Project, asset: Id) -> Result<()> {
        let used = project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .any(|c| c.asset_id == asset);
        if used {
            return Err(StoreError::Other("asset is still used by a clip".into()));
        }
        let idx = project
            .assets
            .iter()
            .position(|a| a.id == asset)
            .ok_or_else(|| StoreError::Other("asset not found".into()))?;
        let path = self.asset_path(&project.assets[idx])?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| StoreError::io(&path, e))?;
        }
        project.assets.remove(idx);
        Ok(())
    }

    /// Corrupt the newest revision on purpose (test helper).
    #[doc(hidden)]
    pub fn debug_corrupt_latest(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE revisions SET doc = substr(doc, 1, length(doc) / 2) WHERE id = (SELECT max(id) FROM revisions)",
            [],
        )?;
        Ok(())
    }
}
