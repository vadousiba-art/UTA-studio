//! Song Lab: practise songs the user legally owns.
//!
//! A song has an optional backing track and an optional reference vocal
//! (both imported by the user), lyric lines with kana readings and timings,
//! phrases (time ranges, usually a few lines) and notes. Practice plays a
//! phrase — optionally looped and slowed down without changing pitch — and
//! records attempts, which are analysed and compared with the reference
//! vocal's pitch contour.
//!
//! Files live in `<data>/songs/<song id>/`: `backing.wav`, `reference.wav`
//! (converted on import) and `cache/` with rate/speed renders.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uta_analysis::compare::{compare_contours, ContourComparison};
use uta_analysis::phrase::{analyze_phrase, PhraseAnalysis};
pub use uta_analysis::pitch::PitchFrame;
use uta_coach::evaluate::{free_singing_feedback, Feedback};
use uta_project::library::{RecordingEntry, SongEntry};

use crate::analyzer::summary_of;
use crate::error::{CoreError, Result};
use crate::romaji::{romanize, Romanized};
use crate::{now, AppCore, CaptureKind};

/// Longest backing track Song Lab accepts (slow-down renders hold the whole
/// track in memory).
pub const MAX_SONG_SECONDS: u64 = 10 * 60;
pub const MIN_SPEED: f32 = 0.5;
const MAX_LINES: usize = 2000;
const MAX_PHRASES: usize = 500;
/// Silence/backing before the phrase when recording an attempt (seconds).
pub const ATTEMPT_PREROLL: f64 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SongTrack {
    Backing,
    Reference,
}

impl SongTrack {
    fn stem(self) -> &'static str {
        match self {
            SongTrack::Backing => "backing",
            SongTrack::Reference => "reference",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongAudio {
    /// File name inside the song folder.
    pub file: String,
    /// Name of the file the user imported (for display only).
    pub original_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

impl SongAudio {
    pub fn seconds(&self) -> f64 {
        self.frames as f64 / self.sample_rate.max(1) as f64
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub id: String,
    /// The lyric as written (kanji and kana).
    pub text: String,
    /// Kana reading (needed for romaji when the line contains kanji).
    #[serde(default)]
    pub reading: String,
    /// Start/end time in the original song, seconds.
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongPhrase {
    pub id: String,
    pub name: String,
    /// Seconds in the original song.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub line_ids: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SongData {
    #[serde(default)]
    pub backing: Option<SongAudio>,
    #[serde(default)]
    pub reference: Option<SongAudio>,
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(default)]
    pub phrases: Vec<SongPhrase>,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_speed")]
    pub practice_speed: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongSummary {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub updated_at: String,
    pub has_backing: bool,
    pub has_reference: bool,
    pub lines: usize,
    pub phrases: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongView {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub created_at: String,
    pub updated_at: String,
    pub data: SongData,
    /// Romaji/mora info per line (from the reading, else the text).
    pub romaji: Vec<Romanized>,
    /// Speeds already rendered for the current device rate (1.0 is always
    /// available when the files match the device rate).
    pub ready_speeds: Vec<f32>,
    pub device_rate: u32,
    pub attempts: Vec<RecordingEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongUpdate {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub lines: Option<Vec<LyricLine>>,
    pub phrases: Option<Vec<SongPhrase>>,
    pub notes: Option<String>,
    pub practice_speed: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongPlayback {
    pub start_frame: u64,
    pub end_frame: u64,
    pub speed: f32,
    pub rate: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongAttemptResult {
    pub recording: RecordingEntry,
    pub analysis: PhraseAnalysis,
    pub feedback: Feedback,
    /// Pitch/timing comparison with the reference vocal over the phrase.
    pub comparison: Option<ContourComparison>,
    /// The attempt's pitch frames mapped to song time (seconds).
    pub contour: Vec<PitchFrame>,
    pub notes: Vec<String>,
}

/// Active song attempt (stored in the capture slot).
#[derive(Debug, Clone)]
pub(crate) struct SongCapture {
    pub song: String,
    pub phrase: String,
    pub speed: f32,
    /// Song time (s) at which the recording started.
    pub song_start: f64,
}

/// Round a requested practice speed to 5 % steps within the allowed range.
pub fn normalize_speed(s: f32) -> f32 {
    if !s.is_finite() {
        return 1.0;
    }
    ((s.clamp(MIN_SPEED, 1.0) * 20.0).round() / 20.0).clamp(MIN_SPEED, 1.0)
}

/// Parse lyrics: LRC (`[mm:ss.xx]text`, several stamps per line allowed,
/// metadata tags ignored) or plain text (one line per lyric line).
pub fn parse_lyrics(text: &str) -> Vec<LyricLine> {
    let mut timed: Vec<(f64, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    let mut any_stamp = false;
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else { break };
            let tag = &stripped[..close];
            if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            } else if tag.contains(':') && !tag.starts_with(|c: char| c.is_ascii_digit()) {
                // metadata such as [ar:...] — ignore the whole line
                stamps.clear();
                rest = "";
                break;
            } else {
                break;
            }
            rest = stripped[close + 1..].trim_start();
        }
        if !stamps.is_empty() {
            any_stamp = true;
            for t in stamps {
                timed.push((t, rest.trim().to_string()));
            }
        } else if !rest.trim().is_empty() {
            plain.push(rest.trim().to_string());
        }
    }
    let new = |text: String, start: Option<f64>| LyricLine { id: uuid::Uuid::new_v4().to_string(), text, reading: String::new(), start, end: None };
    if any_stamp {
        timed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut lines: Vec<LyricLine> = Vec::new();
        for (i, (t, txt)) in timed.iter().enumerate() {
            if txt.is_empty() {
                // an empty stamped line marks the end of the previous line
                if let Some(prev) = lines.last_mut() {
                    prev.end.get_or_insert(*t);
                }
                continue;
            }
            let mut l = new(txt.clone(), Some(*t));
            l.end = timed.get(i + 1).map(|n| n.0);
            lines.push(l);
        }
        lines
    } else {
        plain.into_iter().map(|t| new(t, None)).collect()
    }
}

fn parse_stamp(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let s: f64 = s.trim().parse().ok()?;
    (s.is_finite() && (0.0..60.0).contains(&s)).then_some(m as f64 * 60.0 + s)
}

fn validate(data: &SongData) -> Result<()> {
    if data.lines.len() > MAX_LINES {
        return Err(CoreError::Invalid(format!("at most {MAX_LINES} lyric lines")));
    }
    if data.phrases.len() > MAX_PHRASES {
        return Err(CoreError::Invalid(format!("at most {MAX_PHRASES} phrases")));
    }
    let time_ok = |t: Option<f64>| t.map(|v| v.is_finite() && (0.0..=MAX_SONG_SECONDS as f64 * 2.0).contains(&v)).unwrap_or(true);
    for l in &data.lines {
        if l.text.chars().count() > 500 || l.reading.chars().count() > 500 {
            return Err(CoreError::Invalid("a lyric line is too long".into()));
        }
        if !time_ok(l.start) || !time_ok(l.end) {
            return Err(CoreError::Invalid("lyric times must be within the song".into()));
        }
    }
    for p in &data.phrases {
        if !(p.start.is_finite() && p.end.is_finite() && p.start >= 0.0 && p.end > p.start) {
            return Err(CoreError::Invalid(format!("phrase “{}” needs a start before its end", p.name)));
        }
        if p.end - p.start > 120.0 {
            return Err(CoreError::Invalid("phrases can be at most 2 minutes long".into()));
        }
    }
    Ok(())
}

fn entry_data(e: &SongEntry) -> SongData {
    serde_json::from_value(e.data.clone()).unwrap_or_default()
}

fn speed_tag(speed: f32) -> u32 {
    (normalize_speed(speed) * 100.0).round() as u32
}

impl AppCore {
    fn song_dir(&self, id: &str) -> Result<PathBuf> {
        // ids are UUIDs we generated; refuse anything else (path safety)
        uuid::Uuid::parse_str(id).map_err(|_| CoreError::Invalid("bad song id".into()))?;
        Ok(self.paths.songs_dir.join(id))
    }

    fn song_entry(&self, id: &str) -> Result<SongEntry> {
        self.library
            .lock()
            .songs()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound("song".into()))
    }

    fn device_rate(&self) -> u32 {
        self.audio.lock().engine.as_ref().map(|e| e.sample_rate()).unwrap_or(48_000)
    }

    /// The file to play for `track` at `speed` on the current device, if it
    /// exists (original file when no conversion is needed).
    fn playback_file(&self, id: &str, data: &SongData, track: SongTrack, speed: f32) -> Result<Option<PathBuf>> {
        let audio = match track {
            SongTrack::Backing => data.backing.as_ref(),
            SongTrack::Reference => data.reference.as_ref(),
        };
        let Some(a) = audio else { return Ok(None) };
        if !is_plain_file_name(&a.file) {
            return Err(CoreError::Invalid("damaged song data (file name)".into()));
        }
        let dir = self.song_dir(id)?;
        let rate = self.device_rate();
        let tag = speed_tag(speed);
        if tag == 100 && a.sample_rate == rate {
            return Ok(Some(dir.join(&a.file)));
        }
        let p = dir.join("cache").join(format!("{}-{rate}-{tag}.wav", track.stem()));
        Ok(p.is_file().then_some(p))
    }

    fn view(&self, e: SongEntry) -> Result<SongView> {
        let data = entry_data(&e);
        let romaji = data
            .lines
            .iter()
            .map(|l| romanize(if l.reading.trim().is_empty() { &l.text } else { &l.reading }))
            .collect();
        let mut ready = Vec::new();
        for s in (10..=20).rev().map(|k| k as f32 * 0.05) {
            let b = data.backing.is_none() || self.playback_file(&e.id, &data, SongTrack::Backing, s)?.is_some();
            let r = data.reference.is_none() || self.playback_file(&e.id, &data, SongTrack::Reference, s)?.is_some();
            if b && r && (data.backing.is_some() || data.reference.is_some()) {
                ready.push(normalize_speed(s));
            }
        }
        let prefix = format!("song:{}:", e.id);
        let attempts = self
            .library
            .lock()
            .recordings(1000, Some("songlab"))?
            .into_iter()
            .filter(|r| r.exercise_id.as_deref().map(|x| x.starts_with(&prefix)).unwrap_or(false))
            .collect();
        Ok(SongView {
            id: e.id,
            title: e.title,
            artist: e.artist,
            created_at: e.created_at.to_rfc3339(),
            updated_at: e.updated_at.to_rfc3339(),
            data,
            romaji,
            ready_speeds: ready,
            device_rate: self.device_rate(),
            attempts,
        })
    }

    fn save_song(&self, e: &mut SongEntry, data: &SongData) -> Result<()> {
        validate(data)?;
        e.data = serde_json::to_value(data).map_err(|err| CoreError::Other(err.to_string()))?;
        e.updated_at = now();
        self.library.lock().upsert_song(e)?;
        Ok(())
    }

    pub fn songs(&self) -> Result<Vec<SongSummary>> {
        Ok(self
            .library
            .lock()
            .songs()?
            .iter()
            .map(|e| {
                let d = entry_data(e);
                SongSummary {
                    id: e.id.clone(),
                    title: e.title.clone(),
                    artist: e.artist.clone(),
                    updated_at: e.updated_at.to_rfc3339(),
                    has_backing: d.backing.is_some(),
                    has_reference: d.reference.is_some(),
                    lines: d.lines.len(),
                    phrases: d.phrases.len(),
                }
            })
            .collect())
    }

    pub fn song_create(&self, title: &str, artist: &str) -> Result<SongView> {
        let title = title.trim();
        if title.is_empty() {
            return Err(CoreError::Invalid("the song needs a title".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(self.song_dir(&id)?)?;
        let mut e = SongEntry {
            id: id.clone(),
            title: title.chars().take(120).collect(),
            artist: artist.trim().chars().take(120).collect(),
            data: serde_json::Value::Null,
            created_at: now(),
            updated_at: now(),
        };
        self.save_song(&mut e, &SongData { practice_speed: 1.0, ..Default::default() })?;
        self.view(e)
    }

    pub fn song_get(&self, id: &str) -> Result<SongView> {
        let e = self.song_entry(id)?;
        self.view(e)
    }

    pub fn song_update(&self, id: &str, u: SongUpdate) -> Result<SongView> {
        let mut e = self.song_entry(id)?;
        let mut d = entry_data(&e);
        if let Some(t) = u.title {
            let t = t.trim();
            if t.is_empty() {
                return Err(CoreError::Invalid("the song needs a title".into()));
            }
            e.title = t.chars().take(120).collect();
        }
        if let Some(a) = u.artist {
            e.artist = a.trim().chars().take(120).collect();
        }
        if let Some(l) = u.lines {
            d.lines = l;
        }
        if let Some(p) = u.phrases {
            d.phrases = p;
        }
        if let Some(n) = u.notes {
            d.notes = n.chars().take(20_000).collect();
        }
        if let Some(s) = u.practice_speed {
            d.practice_speed = normalize_speed(s);
        }
        self.save_song(&mut e, &d)?;
        self.view(e)
    }

    /// Replace the lyric lines with lyrics parsed from text (LRC or plain).
    pub fn song_import_lyrics(&self, id: &str, text: &str) -> Result<SongView> {
        if text.len() > 1_000_000 {
            return Err(CoreError::Invalid("that lyrics file is too large".into()));
        }
        let lines = parse_lyrics(text);
        if lines.is_empty() {
            return Err(CoreError::Invalid("no lyric lines found".into()));
        }
        self.song_update(id, SongUpdate { lines: Some(lines), ..Default::default() })
    }

    /// Import lyrics from a UTF-8 `.lrc` or `.txt` file.
    pub fn song_import_lyrics_file(&self, id: &str, path: &Path) -> Result<SongView> {
        let meta = std::fs::metadata(path)?;
        if meta.len() > 1_000_000 {
            return Err(CoreError::Invalid("that lyrics file is too large".into()));
        }
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CoreError::Invalid("the lyrics file is not UTF-8 text (save it as UTF-8 and try again)".into()))?;
        self.song_import_lyrics(id, text.trim_start_matches('\u{feff}'))
    }

    /// Delete a song, its files and its practice attempts.
    pub fn song_delete(&self, id: &str) -> Result<()> {
        let dir = self.song_dir(id)?;
        let prefix = format!("song:{id}:");
        let attempts: Vec<RecordingEntry> = self
            .library
            .lock()
            .recordings(10_000, Some("songlab"))?
            .into_iter()
            .filter(|r| r.exercise_id.as_deref().map(|x| x.starts_with(&prefix)).unwrap_or(false))
            .collect();
        for a in attempts {
            self.delete_recording(&a.id)?;
        }
        self.library.lock().delete_song(id)?;
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Import a backing track or reference vocal (WAV) in the background.
    pub fn song_import_audio(self: &Arc<Self>, id: &str, track: SongTrack, file: PathBuf) -> Result<u64> {
        let dir = self.song_dir(id)?;
        self.song_entry(id)?;
        let info = uta_audio::wav::wav_info(&file)?;
        let secs = info.frames / info.sample_rate.max(1) as u64;
        if secs > MAX_SONG_SECONDS {
            return Err(CoreError::Invalid(format!("Song Lab accepts tracks up to {} minutes; this one is {} min.", MAX_SONG_SECONDS / 60, secs / 60)));
        }
        let rate = self.device_rate();
        let core = self.clone();
        let id = id.to_string();
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        Ok(self.jobs.submit("song-import", format!("Importing {name}"), move |job| {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let tmp = dir.join(format!("{}.part.wav", track.stem()));
            job.set_progress(0.1);
            let out = uta_audio::import::import_wav(&file, &tmp, rate).map_err(|e| e.to_string())?;
            let dest = dir.join(format!("{}.wav", track.stem()));
            std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            // stale renders of this track
            if let Ok(rd) = std::fs::read_dir(dir.join("cache")) {
                for f in rd.flatten() {
                    if f.file_name().to_string_lossy().starts_with(track.stem()) {
                        let _ = std::fs::remove_file(f.path());
                    }
                }
            }
            let mut e = core.song_entry(&id).map_err(|e| e.to_string())?;
            let mut d = entry_data(&e);
            let audio = SongAudio { file: format!("{}.wav", track.stem()), original_name: name, sample_rate: out.sample_rate, channels: out.channels, frames: out.frames };
            match track {
                SongTrack::Backing => d.backing = Some(audio),
                SongTrack::Reference => d.reference = Some(audio),
            }
            core.save_song(&mut e, &d).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "seconds": out.frames as f64 / out.sample_rate as f64 }))
        }))
    }

    /// Remove the backing track or reference vocal.
    pub fn song_remove_audio(&self, id: &str, track: SongTrack) -> Result<SongView> {
        let dir = self.song_dir(id)?;
        let mut e = self.song_entry(id)?;
        let mut d = entry_data(&e);
        match track {
            SongTrack::Backing => d.backing = None,
            SongTrack::Reference => d.reference = None,
        }
        let _ = std::fs::remove_file(dir.join(format!("{}.wav", track.stem())));
        if let Ok(rd) = std::fs::read_dir(dir.join("cache")) {
            for f in rd.flatten() {
                if f.file_name().to_string_lossy().starts_with(track.stem()) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        self.save_song(&mut e, &d)?;
        self.view(e)
    }

    /// Render the song's audio for the current device rate at `speed`
    /// (time-stretched, pitch unchanged) in the background.
    pub fn song_prepare(self: &Arc<Self>, id: &str, speed: f32) -> Result<u64> {
        let speed = normalize_speed(speed);
        let e = self.song_entry(id)?;
        let d = entry_data(&e);
        if d.backing.is_none() && d.reference.is_none() {
            return Err(CoreError::Invalid("import a backing track or reference vocal first".into()));
        }
        let dir = self.song_dir(id)?;
        let rate = self.device_rate();
        let mut work = Vec::new();
        for (track, audio) in [(SongTrack::Backing, &d.backing), (SongTrack::Reference, &d.reference)] {
            if let Some(a) = audio {
                if self.playback_file(id, &d, track, speed)?.is_none() {
                    work.push((track, dir.join(&a.file)));
                }
            }
        }
        let tag = speed_tag(speed);
        Ok(self.jobs.submit("song-prepare", format!("Preparing {}% speed", tag), move |job| {
            let cache = dir.join("cache");
            std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
            let n = work.len().max(1) as f32;
            for (i, (track, src)) in work.into_iter().enumerate() {
                let base = i as f32 / n;
                let buf = uta_audio::wav::read_wav(&src, None).map_err(|e| e.to_string())?;
                let ch = buf.channels.max(1) as usize;
                let samples = uta_audio::wav::resample(&buf.samples, buf.channels, buf.sample_rate, rate).map_err(|e| e.to_string())?;
                if !job.report(base + 0.1 / n) {
                    return Err("cancelled".into());
                }
                let channels: Vec<Vec<f32>> = (0..ch).map(|c| samples.iter().skip(c).step_by(ch).copied().collect()).collect();
                drop(samples);
                let stretched = uta_dsp::stretch::time_stretch(&channels, rate, speed, |p| job.report(base + (0.1 + 0.8 * p) / n))
                    .ok_or_else(|| "cancelled".to_string())?;
                drop(channels);
                let frames = stretched.first().map(|c| c.len()).unwrap_or(0);
                let mut inter = Vec::with_capacity(frames * ch);
                for f in 0..frames {
                    for c in &stretched {
                        inter.push(c[f]);
                    }
                }
                let dest = cache.join(format!("{}-{rate}-{tag}.wav", track.stem()));
                let tmp = dest.with_extension("part.wav");
                uta_audio::wav::write_wav_f32(&tmp, rate, ch as u16, &inter).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            }
            Ok(serde_json::json!({ "speed": speed }))
        }))
    }

    /// Play part of the backing (or reference) from `from`..`to` seconds of
    /// the original song at `speed`, optionally looped.
    pub fn song_play(&self, id: &str, track: SongTrack, from: f64, to: Option<f64>, looped: bool, speed: f32) -> Result<SongPlayback> {
        let speed = normalize_speed(speed);
        let e = self.song_entry(id)?;
        let d = entry_data(&e);
        let file = self.playback_file(id, &d, track, speed)?.ok_or_else(|| {
            CoreError::Invalid(match track {
                SongTrack::Backing if d.backing.is_none() => "this song has no backing track yet".into(),
                SongTrack::Reference if d.reference.is_none() => "this song has no reference vocal yet".into(),
                _ => format!("prepare {}% speed first", speed_tag(speed)),
            })
        })?;
        let rate = self.device_rate();
        let to_frame = |t: f64| ((t.max(0.0) / speed as f64) * rate as f64).round() as u64;
        let start = to_frame(from);
        let end = to.map(to_frame);
        if let Some(e) = end {
            if e <= start {
                return Err(CoreError::Invalid("the end must be after the start".into()));
            }
        }
        let end_frame = self.with_engine(|e| Ok(e.preview_range(file, start, end, looped, 1.0)?))?;
        Ok(SongPlayback { start_frame: start, end_frame, speed, rate })
    }

    pub fn song_stop(&self) -> Result<()> {
        self.stop_preview()
    }

    /// Start recording an attempt at a phrase: the backing plays from
    /// [`ATTEMPT_PREROLL`] seconds before the phrase (if the song has one)
    /// while the microphone records. Use headphones so the backing doesn't
    /// leak into the recording.
    pub fn song_attempt_start(&self, id: &str, phrase_id: &str, speed: f32, with_backing: bool) -> Result<SongPlayback> {
        let speed = normalize_speed(speed);
        let e = self.song_entry(id)?;
        let d = entry_data(&e);
        let phrase = d.phrases.iter().find(|p| p.id == phrase_id).ok_or_else(|| CoreError::NotFound("phrase".into()))?.clone();
        let rate = self.device_rate();
        let song_start = (phrase.start - ATTEMPT_PREROLL).max(0.0);
        let backing = if with_backing && d.backing.is_some() {
            Some(self.playback_file(id, &d, SongTrack::Backing, speed)?.ok_or_else(|| CoreError::Invalid(format!("prepare {}% speed first", speed_tag(speed))))?)
        } else {
            None
        };
        let to_frame = |t: f64| ((t / speed as f64) * rate as f64).round() as u64;
        let (start, end) = (to_frame(song_start), to_frame(phrase.end + 0.5));
        let cap = SongCapture { song: id.to_string(), phrase: phrase.id.clone(), speed, song_start };
        let after: Option<crate::analyzer::EngineHook> = backing.map(|file| {
            Box::new(move |eng: &mut uta_audio::AudioEngine| {
                eng.preview_range(file, start, Some(end), false, 1.0)?;
                Ok(())
            }) as crate::analyzer::EngineHook
        });
        self.begin_capture_with(CaptureKind::Song(Box::new(cap)), None, after)?;
        Ok(SongPlayback { start_frame: start, end_frame: end, speed, rate })
    }

    /// Stop the attempt, analyse it and compare it with the reference vocal.
    pub fn song_attempt_stop(&self) -> Result<SongAttemptResult> {
        match self.capture.lock().as_ref() {
            Some(c) if matches!(c.kind, CaptureKind::Song(_)) => {}
            Some(_) => return Err(CoreError::Invalid("the active recording is not a Song Lab attempt".into())),
            None => return Err(CoreError::Invalid("not recording".into())),
        }
        let backing_offset = self.with_engine(|e| Ok(e.shared().preview_start_record_frame.load(std::sync::atomic::Ordering::Relaxed))).unwrap_or(u64::MAX);
        let _ = self.stop_preview();
        let (cap, mono, sr) = self.end_capture()?;
        // seconds of recording before the backing actually started
        let offset_s = if backing_offset == u64::MAX { 0.0 } else { backing_offset as f64 / sr.max(1) as f64 };
        let CaptureKind::Song(sc) = &cap.kind else {
            return Err(CoreError::Invalid("the active recording is not a Song Lab attempt".into()));
        };
        let e = self.song_entry(&sc.song)?;
        let d = entry_data(&e);
        let phrase = d.phrases.iter().find(|p| p.id == sc.phrase).cloned();
        let opts = self.analysis_options();
        let analysis = analyze_phrase(&mono, sr, &opts);
        let feedback = free_singing_feedback(&analysis);
        let mut notes = Vec::new();
        // map attempt frames to song time (latency removed, speed undone)
        let contour: Vec<PitchFrame> = analysis
            .frames
            .iter()
            .map(|f| {
                let mut g = *f;
                g.time = (sc.song_start + (f.time as f64 - cap.latency_s as f64 - offset_s).max(0.0) * sc.speed as f64) as f32;
                g
            })
            .collect();
        let mut comparison = None;
        if let (Some(p), Some(_)) = (&phrase, &d.reference) {
            match self.reference_contour(&sc.song, &d, p) {
                Ok(reference) => {
                    let take: Vec<PitchFrame> = contour.iter().filter(|f| (f.time as f64) >= p.start - 0.3 && (f.time as f64) <= p.end + 0.3).copied().collect();
                    let voiced = |fr: &[PitchFrame]| fr.iter().filter(|f| f.hz.is_some()).count();
                    // comparing silence (or a few stray frames) would produce meaningless numbers
                    comparison = if voiced(&take) * 4 >= voiced(&reference).max(8) {
                        compare_contours(&take, &reference, 1.5).filter(|c| c.voiced_overlap >= 0.2)
                    } else {
                        None
                    };
                    if comparison.is_none() {
                        notes.push("Not enough singing in the phrase to compare with the reference.".into());
                    }
                }
                Err(err) => notes.push(format!("The reference vocal could not be analysed: {err}")),
            }
        } else if d.reference.is_none() {
            notes.push("Import a reference vocal to compare pitch and timing with it.".into());
        }
        if sc.speed < 1.0 {
            notes.push(format!("Recorded at {}% speed; times are shown in song time.", speed_tag(sc.speed)));
        }
        let phrase_name = phrase.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Phrase".into());
        let exercise_id = format!("song:{}:{}", sc.song, sc.phrase);
        let n = self
            .library
            .lock()
            .recordings(10_000, Some("songlab"))?
            .iter()
            .filter(|r| r.exercise_id.as_deref() == Some(&exercise_id))
            .count();
        let mut summary = summary_of(&analysis);
        if let Some(c) = &comparison {
            summary["refPitchDiffCents"] = serde_json::json!(c.mean_abs_pitch_diff_cents);
            summary["refTimingMs"] = serde_json::json!(c.median_timing_offset_ms);
        }
        summary["speed"] = serde_json::json!(sc.speed);
        let entry = RecordingEntry {
            id: uuid::Uuid::new_v4().to_string(),
            path: cap.path.to_string_lossy().to_string(),
            created_at: cap.started,
            duration: analysis.duration as f64,
            source: "songlab".into(),
            title: format!("{} · {} · take {}", e.title, phrase_name, n + 1),
            exercise_id: Some(exercise_id),
            summary,
        };
        if let Ok(json) = serde_json::to_vec(&analysis) {
            let _ = std::fs::write(cap.path.with_extension("analysis.json"), json);
        }
        self.library.lock().add_recording(&entry)?;
        Ok(SongAttemptResult { recording: entry, analysis, feedback, comparison, contour, notes })
    }

    fn reference_contour(&self, id: &str, d: &SongData, p: &SongPhrase) -> Result<Vec<PitchFrame>> {
        let a = d.reference.as_ref().ok_or_else(|| CoreError::Invalid("no reference vocal".into()))?;
        if !is_plain_file_name(&a.file) {
            return Err(CoreError::Invalid("damaged song data (file name)".into()));
        }
        let path = self.song_dir(id)?.join(&a.file);
        let mut rd = uta_audio::wav::WavStreamReader::open(&path)?;
        let sr = rd.info.sample_rate;
        let ch = rd.info.channels.max(1) as usize;
        let start = (p.start * sr as f64) as u64;
        let frames = ((p.end - p.start) * sr as f64) as usize;
        let mut tmp = Vec::new();
        rd.read(start, frames, &mut tmp).map_err(|e| CoreError::Other(e.to_string()))?;
        let mono: Vec<f32> = tmp.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
        let analysis = analyze_phrase(&mono, sr, &self.analysis_options());
        Ok(analysis
            .frames
            .into_iter()
            .map(|mut f| {
                f.time += p.start as f32;
                f
            })
            .collect())
    }

    /// Pitch contour of the reference vocal over a phrase (song time).
    pub fn song_reference_contour(&self, id: &str, phrase_id: &str) -> Result<Vec<PitchFrame>> {
        let e = self.song_entry(id)?;
        let d = entry_data(&e);
        let p = d.phrases.iter().find(|p| p.id == phrase_id).ok_or_else(|| CoreError::NotFound("phrase".into()))?;
        self.reference_contour(id, &d, p)
    }
}

/// Paths in song data are file names we wrote; keep them that way.
pub(crate) fn is_plain_file_name(s: &str) -> bool {
    !s.is_empty() && Path::new(s).file_name().map(|n| n == s).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_and_plain_lyrics() {
        let lrc = "[ar:Someone]\n[ti:Song]\n[00:12.50]さくら さくら\n[00:15.00][00:40.00]やよいの そらは\n[00:18.25]\n";
        let l = parse_lyrics(lrc);
        assert_eq!(l.len(), 3, "{l:?}");
        assert_eq!(l[0].text, "さくら さくら");
        assert_eq!(l[0].start, Some(12.5));
        assert_eq!(l[0].end, Some(15.0));
        assert_eq!(l[1].start, Some(15.0));
        assert_eq!(l[1].end, Some(18.25), "empty stamped line ends the previous one");
        assert_eq!(l[2].text, "やよいの そらは");
        assert_eq!(l[2].start, Some(40.0));
        let p = parse_lyrics("一行目\n\n  二行目  \n");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].text, "二行目");
        assert!(p[0].start.is_none());
    }

    #[test]
    fn speeds_are_normalised() {
        assert_eq!(normalize_speed(0.73), 0.75);
        assert_eq!(normalize_speed(0.1), MIN_SPEED);
        assert_eq!(normalize_speed(3.0), 1.0);
        assert_eq!(normalize_speed(f32::NAN), 1.0);
        assert_eq!(speed_tag(0.75), 75);
    }

    #[test]
    fn plain_file_names_only() {
        assert!(is_plain_file_name("backing.wav"));
        assert!(!is_plain_file_name("../x.wav"));
        assert!(!is_plain_file_name("a/b.wav"));
    }
}
