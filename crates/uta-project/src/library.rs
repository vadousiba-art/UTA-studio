//! The app-wide library database (`library.sqlite` in the app data folder):
//! settings, recent projects, practice recordings, training history, skill
//! levels, review schedule, Song Lab songs, effect presets and the plugin
//! scan cache.
//!
//! Schema changes are applied by numbered migrations tracked in SQLite's
//! `user_version` pragma.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::error::StoreError;

type Result<T> = std::result::Result<T, StoreError>;

const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
     CREATE TABLE projects (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
        created_at TEXT NOT NULL, modified_at TEXT NOT NULL, last_opened_at TEXT NOT NULL);
     CREATE TABLE recordings (
        id TEXT PRIMARY KEY, path TEXT NOT NULL, created_at TEXT NOT NULL,
        duration REAL NOT NULL, source TEXT NOT NULL, title TEXT NOT NULL,
        exercise_id TEXT, summary TEXT NOT NULL DEFAULT '{}');
     CREATE INDEX recordings_created ON recordings(created_at);
     CREATE TABLE attempts (
        id TEXT PRIMARY KEY, created_at TEXT NOT NULL, exercise_id TEXT NOT NULL,
        skill_id TEXT NOT NULL, level INTEGER NOT NULL, outcome TEXT NOT NULL,
        score REAL, confidence REAL, metrics TEXT NOT NULL DEFAULT '{}',
        recording_id TEXT, user_difficulty INTEGER, discomfort INTEGER NOT NULL DEFAULT 0,
        notes TEXT NOT NULL DEFAULT '');
     CREATE INDEX attempts_skill ON attempts(skill_id, created_at);
     CREATE INDEX attempts_exercise ON attempts(exercise_id, created_at);
     CREATE TABLE skills (skill_id TEXT PRIMARY KEY, level INTEGER NOT NULL, updated_at TEXT NOT NULL, data TEXT NOT NULL DEFAULT '{}');
     CREATE TABLE reviews (
        exercise_id TEXT PRIMARY KEY, skill_id TEXT NOT NULL, box INTEGER NOT NULL,
        due TEXT NOT NULL, last_practiced TEXT NOT NULL, successes INTEGER NOT NULL DEFAULT 0,
        last_outcome TEXT NOT NULL);
     CREATE TABLE songs (
        id TEXT PRIMARY KEY, title TEXT NOT NULL, artist TEXT NOT NULL DEFAULT '',
        data TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
     CREATE TABLE presets (
        id TEXT PRIMARY KEY, effect_kind TEXT NOT NULL, name TEXT NOT NULL,
        params TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE(effect_kind, name));
     CREATE TABLE plugin_cache (
        path TEXT PRIMARY KEY, modified INTEGER NOT NULL, data TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1, scanned_at TEXT NOT NULL);
     CREATE TABLE notes (id TEXT PRIMARY KEY, created_at TEXT NOT NULL, skill_id TEXT, text TEXT NOT NULL);",
];

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn parse_time(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn json_col<T: DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub last_opened_at: DateTime<Utc>,
    /// Filled in by queries: whether the folder still exists.
    #[serde(default)]
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEntry {
    pub id: String,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub duration: f64,
    /// `analyzer`, `coach`, `songlab`.
    pub source: String,
    pub title: String,
    pub exercise_id: Option<String>,
    /// Small JSON summary of the analysis (for lists and comparisons).
    pub summary: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub exercise_id: String,
    pub skill_id: String,
    pub level: u32,
    /// `success`, `partial`, `needsWork`, `inconclusive`, `stopped`.
    pub outcome: String,
    pub score: Option<f32>,
    pub confidence: Option<f32>,
    pub metrics: Value,
    pub recording_id: Option<String>,
    /// Self-reported difficulty 1 (easy) – 5 (very hard).
    pub user_difficulty: Option<u8>,
    pub discomfort: bool,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub skill_id: String,
    pub level: u32,
    pub updated_at: DateTime<Utc>,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEntry {
    pub exercise_id: String,
    pub skill_id: String,
    /// Leitner box index (0 = review soon).
    pub box_index: u32,
    pub due: DateTime<Utc>,
    pub last_practiced: DateTime<Utc>,
    pub successes: u32,
    pub last_outcome: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongEntry {
    pub id: String,
    pub title: String,
    pub artist: String,
    /// Song Lab data (backing path, lyric lines, loops, speed, notes).
    pub data: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetEntry {
    pub id: String,
    pub effect_kind: String,
    pub name: String,
    pub params: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCacheEntry {
    pub path: String,
    pub modified: i64,
    pub data: Value,
    pub enabled: bool,
    pub scanned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub skill_id: Option<String>,
    pub text: String,
}

pub struct Library {
    conn: Connection,
    path: PathBuf,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::io(parent, e))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, path.to_path_buf())
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, PathBuf::from(":memory:"))
    }

    fn init(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        let mut lib = Self { conn, path };
        lib.migrate()?;
        Ok(lib)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version as usize > MIGRATIONS.len() {
            return Err(StoreError::TooNew {
                found: version as u32,
                supported: MIGRATIONS.len() as u32,
            });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    // ------------------------------------------------------------ settings

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(match v {
            Some(s) => serde_json::from_str(&s).ok(),
            None => None,
        })
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ projects

    pub fn touch_project(&self, id: &str, name: &str, path: &str, created_at: DateTime<Utc>, modified_at: DateTime<Utc>) -> Result<()> {
        // a path can only belong to one entry
        self.conn.execute("DELETE FROM projects WHERE path = ?1 AND id != ?2", params![path, id])?;
        self.conn.execute(
            "INSERT INTO projects(id, name, path, created_at, modified_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, path = excluded.path,
               modified_at = excluded.modified_at, last_opened_at = excluded.last_opened_at",
            params![id, name, path, created_at.to_rfc3339(), modified_at.to_rfc3339(), now()],
        )?;
        Ok(())
    }

    pub fn recent_projects(&self, limit: usize, query: Option<&str>) -> Result<Vec<ProjectEntry>> {
        let pattern = format!("%{}%", query.unwrap_or("").replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, created_at, modified_at, last_opened_at FROM projects
             WHERE name LIKE ?1 ESCAPE '\\' ORDER BY last_opened_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |r| {
            let path: String = r.get(2)?;
            Ok(ProjectEntry {
                id: r.get(0)?,
                name: r.get(1)?,
                exists: Path::new(&path).is_dir(),
                path,
                created_at: parse_time(&r.get::<_, String>(3)?),
                modified_at: parse_time(&r.get::<_, String>(4)?),
                last_opened_at: parse_time(&r.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn forget_project(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------------------------------------------------------- recordings

    pub fn add_recording(&self, e: &RecordingEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO recordings(id, path, created_at, duration, source, title, exercise_id, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                e.id,
                e.path,
                e.created_at.to_rfc3339(),
                e.duration,
                e.source,
                e.title,
                e.exercise_id,
                serde_json::to_string(&e.summary)?
            ],
        )?;
        Ok(())
    }

    fn recording_from_row(r: &Row) -> rusqlite::Result<RecordingEntry> {
        Ok(RecordingEntry {
            id: r.get(0)?,
            path: r.get(1)?,
            created_at: parse_time(&r.get::<_, String>(2)?),
            duration: r.get(3)?,
            source: r.get(4)?,
            title: r.get(5)?,
            exercise_id: r.get(6)?,
            summary: json_col::<Value>(&r.get::<_, String>(7)?),
        })
    }

    pub fn recordings(&self, limit: usize, source: Option<&str>) -> Result<Vec<RecordingEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, created_at, duration, source, title, exercise_id, summary FROM recordings
             WHERE (?1 IS NULL OR source = ?1) ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![source, limit as i64], Self::recording_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn recording(&self, id: &str) -> Result<Option<RecordingEntry>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, created_at, duration, source, title, exercise_id, summary FROM recordings WHERE id = ?1",
                params![id],
                Self::recording_from_row,
            )
            .optional()?)
    }

    pub fn rename_recording(&self, id: &str, title: &str) -> Result<()> {
        self.conn
            .execute("UPDATE recordings SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    /// Delete a recording row and its audio file (only files inside
    /// `recordings_dir` are ever deleted).
    pub fn delete_recording(&self, id: &str, recordings_dir: &Path) -> Result<()> {
        if let Some(rec) = self.recording(id)? {
            let p = PathBuf::from(&rec.path);
            let inside = p
                .canonicalize()
                .ok()
                .zip(recordings_dir.canonicalize().ok())
                .map(|(f, d)| f.starts_with(d))
                .unwrap_or(false);
            if inside && p.is_file() {
                std::fs::remove_file(&p).map_err(|e| StoreError::io(&p, e))?;
            }
            self.conn.execute("DELETE FROM recordings WHERE id = ?1", params![id])?;
            self.conn
                .execute("UPDATE attempts SET recording_id = NULL WHERE recording_id = ?1", params![id])?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ attempts

    pub fn add_attempt(&self, a: &AttemptEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO attempts(id, created_at, exercise_id, skill_id, level, outcome, score, confidence,
                                  metrics, recording_id, user_difficulty, discomfort, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                a.id,
                a.created_at.to_rfc3339(),
                a.exercise_id,
                a.skill_id,
                a.level,
                a.outcome,
                a.score,
                a.confidence,
                serde_json::to_string(&a.metrics)?,
                a.recording_id,
                a.user_difficulty,
                a.discomfort as i64,
                a.notes
            ],
        )?;
        Ok(())
    }

    fn attempt_from_row(r: &Row) -> rusqlite::Result<AttemptEntry> {
        Ok(AttemptEntry {
            id: r.get(0)?,
            created_at: parse_time(&r.get::<_, String>(1)?),
            exercise_id: r.get(2)?,
            skill_id: r.get(3)?,
            level: r.get(4)?,
            outcome: r.get(5)?,
            score: r.get(6)?,
            confidence: r.get(7)?,
            metrics: json_col::<Value>(&r.get::<_, String>(8)?),
            recording_id: r.get(9)?,
            user_difficulty: r.get(10)?,
            discomfort: r.get::<_, i64>(11)? != 0,
            notes: r.get(12)?,
        })
    }

    const ATTEMPT_COLS: &'static str = "id, created_at, exercise_id, skill_id, level, outcome, score, confidence, metrics, recording_id, user_difficulty, discomfort, notes";

    /// Most recent attempts first. Filters are optional.
    pub fn attempts(&self, skill: Option<&str>, exercise: Option<&str>, limit: usize) -> Result<Vec<AttemptEntry>> {
        let sql = format!(
            "SELECT {} FROM attempts WHERE (?1 IS NULL OR skill_id = ?1) AND (?2 IS NULL OR exercise_id = ?2)
             ORDER BY created_at DESC, rowid DESC LIMIT ?3",
            Self::ATTEMPT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![skill, exercise, limit as i64], Self::attempt_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn attempts_since(&self, since: DateTime<Utc>) -> Result<Vec<AttemptEntry>> {
        let sql = format!(
            "SELECT {} FROM attempts WHERE created_at >= ?1 ORDER BY created_at ASC",
            Self::ATTEMPT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since.to_rfc3339()], Self::attempt_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_attempt(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM attempts WHERE id = ?1", params![id])?;
        Ok(())
    }

    // -------------------------------------------------------------- skills

    pub fn skills(&self) -> Result<Vec<SkillEntry>> {
        let mut stmt = self.conn.prepare("SELECT skill_id, level, updated_at, data FROM skills")?;
        let rows = stmt.query_map([], |r| {
            Ok(SkillEntry {
                skill_id: r.get(0)?,
                level: r.get(1)?,
                updated_at: parse_time(&r.get::<_, String>(2)?),
                data: json_col::<Value>(&r.get::<_, String>(3)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn set_skill(&self, skill_id: &str, level: u32, data: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO skills(skill_id, level, updated_at, data) VALUES (?1, ?2, ?3, ?4)",
            params![skill_id, level, now(), serde_json::to_string(data)?],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------- reviews

    pub fn reviews(&self) -> Result<Vec<ReviewEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT exercise_id, skill_id, box, due, last_practiced, successes, last_outcome FROM reviews ORDER BY due ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ReviewEntry {
                exercise_id: r.get(0)?,
                skill_id: r.get(1)?,
                box_index: r.get(2)?,
                due: parse_time(&r.get::<_, String>(3)?),
                last_practiced: parse_time(&r.get::<_, String>(4)?),
                successes: r.get(5)?,
                last_outcome: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn upsert_review(&self, e: &ReviewEntry) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO reviews(exercise_id, skill_id, box, due, last_practiced, successes, last_outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                e.exercise_id,
                e.skill_id,
                e.box_index,
                e.due.to_rfc3339(),
                e.last_practiced.to_rfc3339(),
                e.successes,
                e.last_outcome
            ],
        )?;
        Ok(())
    }

    // --------------------------------------------------------------- songs

    pub fn songs(&self) -> Result<Vec<SongEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, artist, data, created_at, updated_at FROM songs ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(SongEntry {
                id: r.get(0)?,
                title: r.get(1)?,
                artist: r.get(2)?,
                data: json_col::<Value>(&r.get::<_, String>(3)?),
                created_at: parse_time(&r.get::<_, String>(4)?),
                updated_at: parse_time(&r.get::<_, String>(5)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn upsert_song(&self, s: &SongEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO songs(id, title, artist, data, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, artist = excluded.artist,
               data = excluded.data, updated_at = excluded.updated_at",
            params![
                s.id,
                s.title,
                s.artist,
                serde_json::to_string(&s.data)?,
                s.created_at.to_rfc3339(),
                now()
            ],
        )?;
        Ok(())
    }

    pub fn delete_song(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM songs WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ------------------------------------------------------------- presets

    pub fn presets(&self, effect_kind: Option<&str>) -> Result<Vec<PresetEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, effect_kind, name, params, created_at FROM presets WHERE (?1 IS NULL OR effect_kind = ?1) ORDER BY name",
        )?;
        let rows = stmt.query_map(params![effect_kind], |r| {
            Ok(PresetEntry {
                id: r.get(0)?,
                effect_kind: r.get(1)?,
                name: r.get(2)?,
                params: json_col::<Value>(&r.get::<_, String>(3)?),
                created_at: parse_time(&r.get::<_, String>(4)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Save (or overwrite by name) a preset.
    pub fn save_preset(&self, p: &PresetEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO presets(id, effect_kind, name, params, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(effect_kind, name) DO UPDATE SET params = excluded.params",
            params![p.id, p.effect_kind, p.name, serde_json::to_string(&p.params)?, p.created_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn delete_preset(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM presets WHERE id = ?1", params![id])?;
        Ok(())
    }

    // -------------------------------------------------------- plugin cache

    pub fn plugin_cache(&self) -> Result<Vec<PluginCacheEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, modified, data, enabled, scanned_at FROM plugin_cache ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(PluginCacheEntry {
                path: r.get(0)?,
                modified: r.get(1)?,
                data: json_col::<Value>(&r.get::<_, String>(2)?),
                enabled: r.get::<_, i64>(3)? != 0,
                scanned_at: parse_time(&r.get::<_, String>(4)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn upsert_plugin_cache(&self, e: &PluginCacheEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO plugin_cache(path, modified, data, enabled, scanned_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET modified = excluded.modified, data = excluded.data, scanned_at = excluded.scanned_at",
            params![e.path, e.modified, serde_json::to_string(&e.data)?, e.enabled as i64, e.scanned_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn set_plugin_enabled(&self, path: &str, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE plugin_cache SET enabled = ?2 WHERE path = ?1",
            params![path, enabled as i64],
        )?;
        Ok(())
    }

    pub fn remove_plugin_cache(&self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM plugin_cache WHERE path = ?1", params![path])?;
        Ok(())
    }

    // --------------------------------------------------------------- notes

    pub fn add_note(&self, n: &NoteEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO notes(id, created_at, skill_id, text) VALUES (?1, ?2, ?3, ?4)",
            params![n.id, n.created_at.to_rfc3339(), n.skill_id, n.text],
        )?;
        Ok(())
    }

    pub fn notes(&self, limit: usize) -> Result<Vec<NoteEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, created_at, skill_id, text FROM notes ORDER BY created_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(NoteEntry {
                id: r.get(0)?,
                created_at: parse_time(&r.get::<_, String>(1)?),
                skill_id: r.get(2)?,
                text: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_note(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ------------------------------------------------- privacy / backup

    /// Delete all training history (attempts, skills, reviews, notes). Does not
    /// touch projects or recordings.
    pub fn delete_training_history(&self) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM attempts; DELETE FROM skills; DELETE FROM reviews; DELETE FROM notes;")?;
        Ok(())
    }

    /// Write a consistent copy of the library database to `dest`.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(StoreError::AlreadyExists(dest.to_path_buf()));
        }
        self.conn
            .execute("VACUUM INTO ?1", params![dest.to_string_lossy().to_string()])?;
        Ok(())
    }
}
