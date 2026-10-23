//! Track sources: the list of audio regions each track plays, derived from the
//! project's clips and take-group comps, plus a reader that renders any
//! timeline range of a track from the asset files.
//!
//! Used by the streaming thread (live playback) and the offline renderer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use uta_dsp::db_to_gain;
use uta_project::{Asset, Project, Track};

use crate::wav::WavStreamReader;

/// Crossfade applied at boundaries between comp segments (frames).
pub const COMP_CROSSFADE: u64 = 128;

/// One contiguous region of an asset placed on the timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRegion {
    /// Index into [`SourceSet::files`].
    pub file: usize,
    pub start: u64,
    pub len: u64,
    pub src_offset: u64,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
}

impl PlayRegion {
    pub fn end(&self) -> u64 {
        self.start + self.len
    }

    /// Gain envelope (clip gain × fades) at timeline frame `t`.
    #[inline]
    fn envelope(&self, t: u64) -> f32 {
        let rel = t - self.start;
        let mut g = self.gain;
        if self.fade_in > 0 && rel < self.fade_in {
            g *= rel as f32 / self.fade_in as f32;
        }
        let to_end = self.len - rel;
        if self.fade_out > 0 && to_end <= self.fade_out {
            g *= (to_end - 1) as f32 / self.fade_out as f32;
        }
        g
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrackSource {
    pub id: u128,
    /// Sorted by start.
    pub regions: Vec<PlayRegion>,
}

#[derive(Debug, Clone, Default)]
pub struct SourceSet {
    pub files: Vec<PathBuf>,
    pub tracks: Vec<TrackSource>,
    /// Assets that could not be resolved (missing or unsafe path).
    pub missing_assets: Vec<uuid::Uuid>,
}

impl SourceSet {
    pub fn track(&self, id: u128) -> Option<&TrackSource> {
        self.tracks.iter().find(|t| t.id == id)
    }
}

/// Build sources for all tracks. `resolve` maps an asset to an existing file
/// path or `None` if the asset is missing (its regions are skipped).
pub fn build_sources(project: &Project, resolve: impl Fn(&Asset) -> Option<PathBuf>) -> SourceSet {
    let mut set = SourceSet::default();
    let mut file_index: HashMap<uuid::Uuid, Option<usize>> = HashMap::new();
    for a in &project.assets {
        let idx = match resolve(a) {
            Some(p) => {
                set.files.push(p);
                Some(set.files.len() - 1)
            }
            None => {
                set.missing_assets.push(a.id);
                None
            }
        };
        file_index.insert(a.id, idx);
    }
    for t in &project.tracks {
        set.tracks.push(TrackSource {
            id: t.id.as_u128(),
            regions: track_regions(t, &file_index),
        });
    }
    set
}

fn track_regions(t: &Track, files: &HashMap<uuid::Uuid, Option<usize>>) -> Vec<PlayRegion> {
    let mut out = Vec::new();
    let in_group = |id: uuid::Uuid| t.take_groups.iter().any(|g| g.takes.contains(&id));
    for c in &t.clips {
        if c.muted || in_group(c.id) {
            continue;
        }
        if let Some(Some(f)) = files.get(&c.asset_id) {
            out.push(PlayRegion {
                file: *f,
                start: c.position,
                len: c.length,
                src_offset: c.source_offset,
                gain: db_to_gain(c.gain_db),
                fade_in: c.fade_in,
                fade_out: c.fade_out,
            });
        }
    }
    for g in &t.take_groups {
        let mut segs = g.comp.clone();
        segs.sort_by_key(|s| s.start);
        for (i, s) in segs.iter().enumerate() {
            let Some(c) = t.clips.iter().find(|c| c.id == s.take) else { continue };
            if c.muted {
                continue;
            }
            let Some(Some(f)) = files.get(&c.asset_id) else { continue };
            let joins_prev = i > 0 && segs[i - 1].end == s.start;
            let joins_next = i + 1 < segs.len() && segs[i + 1].start == s.end;
            let half = COMP_CROSSFADE / 2;
            // extend into neighbours for an overlapping crossfade, within the
            // take's own audio
            let start = if joins_prev { s.start.saturating_sub(half).max(c.position) } else { s.start };
            let end = if joins_next { (s.end + half).min(c.end()) } else { s.end };
            if end <= start {
                continue;
            }
            let fade_in = if joins_prev {
                s.start - start + half
            } else if start == c.position {
                c.fade_in
            } else {
                0
            };
            let fade_out = if joins_next {
                end - s.end + half
            } else if end == c.end() {
                c.fade_out
            } else {
                0
            };
            let len = end - start;
            out.push(PlayRegion {
                file: *f,
                start,
                len,
                src_offset: c.source_offset + (start - c.position),
                gain: db_to_gain(c.gain_db),
                fade_in: fade_in.min(len),
                fade_out: fade_out.min(len),
            });
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Renders track audio from files. Keeps a bounded cache of open readers.
pub struct SourceReader {
    set: Arc<SourceSet>,
    readers: HashMap<usize, WavStreamReader>,
    order: Vec<usize>,
    max_open: usize,
    tmp: Vec<f32>,
    /// Errors encountered (file index, message), for reporting.
    pub errors: Vec<(usize, String)>,
}

impl SourceReader {
    pub fn new(set: Arc<SourceSet>) -> Self {
        Self {
            set,
            readers: HashMap::new(),
            order: Vec::new(),
            max_open: 48,
            tmp: Vec::with_capacity(8192),
            errors: Vec::new(),
        }
    }

    pub fn set(&self) -> &Arc<SourceSet> {
        &self.set
    }

    fn reader(&mut self, file: usize) -> Option<&mut WavStreamReader> {
        if !self.readers.contains_key(&file) {
            let path = self.set.files.get(file)?.clone();
            match WavStreamReader::open(&path) {
                Ok(r) => {
                    if self.readers.len() >= self.max_open {
                        if let Some(old) = self.order.first().copied() {
                            self.readers.remove(&old);
                            self.order.remove(0);
                        }
                    }
                    self.readers.insert(file, r);
                    self.order.push(file);
                }
                Err(e) => {
                    if !self.errors.iter().any(|(f, _)| *f == file) {
                        self.errors.push((file, e.to_string()));
                    }
                    return None;
                }
            }
        }
        self.readers.get_mut(&file)
    }

    /// Render `l.len()` frames of track `index` starting at timeline frame
    /// `start`. Overwrites `l` and `r`.
    pub fn render(&mut self, index: usize, start: u64, l: &mut [f32], r: &mut [f32]) {
        l.fill(0.0);
        r.fill(0.0);
        let n = l.len() as u64;
        let end = start + n;
        let set = self.set.clone();
        let Some(track) = set.tracks.get(index) else { return };
        // regions are sorted by start; skip those that end before `start`
        for reg in track.regions.iter().filter(|g| g.start < end && g.end() > start) {
            let from = start.max(reg.start);
            let to = end.min(reg.end());
            let frames = (to - from) as usize;
            let src = reg.src_offset + (from - reg.start);
            let mut tmp = std::mem::take(&mut self.tmp);
            let (got, channels) = match self.reader(reg.file) {
                Some(rd) => {
                    let ch = rd.info.channels as usize;
                    match rd.read(src, frames, &mut tmp) {
                        Ok(g) => (g, ch),
                        Err(e) => {
                            self.errors.push((reg.file, e.to_string()));
                            (0, ch)
                        }
                    }
                }
                None => (0, 1),
            };
            let base = (from - start) as usize;
            for k in 0..got {
                let t = from + k as u64;
                let g = reg.envelope(t);
                let (a, b) = if channels == 1 {
                    let s = tmp[k];
                    (s, s)
                } else {
                    (tmp[k * channels], tmp[k * channels + 1])
                };
                l[base + k] += a * g;
                r[base + k] += b * g;
            }
            self.tmp = tmp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uta_project::{edit, new_id, AssetKind};

    fn setup(dir: &std::path::Path) -> (Project, uuid::Uuid, uuid::Uuid) {
        let mut p = Project::new("s", 48_000);
        let t = edit::add_track(&mut p, None);
        // asset: ramp 0..1 over 1000 frames, mono
        let sig: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        std::fs::create_dir_all(dir.join("audio")).unwrap();
        crate::wav::write_wav_f32(&dir.join("audio/a.wav"), 48_000, 1, &sig).unwrap();
        let a = Asset {
            id: new_id(),
            file: "audio/a.wav".into(),
            original_name: "a".into(),
            kind: AssetKind::Recording,
            sample_rate: 48_000,
            channels: 1,
            frames: 1000,
            created_at: Utc::now(),
        };
        let aid = a.id;
        p.assets.push(a);
        (p, t, aid)
    }

    fn resolver(dir: std::path::PathBuf) -> impl Fn(&Asset) -> Option<PathBuf> {
        move |a: &Asset| {
            let p = dir.join(&a.file);
            p.is_file().then_some(p)
        }
    }

    #[test]
    fn renders_clip_at_position_with_offset() {
        let d = tempfile::tempdir().unwrap();
        let (mut p, t, aid) = setup(d.path());
        let c = edit::add_clip(&mut p, t, aid, 100).unwrap();
        edit::trim_clip(&mut p, c, Some(200), Some(600)).unwrap();
        let set = Arc::new(build_sources(&p, resolver(d.path().to_path_buf())));
        let mut rd = SourceReader::new(set);
        let (mut l, mut r) = (vec![0.0; 1000], vec![0.0; 1000]);
        rd.render(0, 0, &mut l, &mut r);
        assert_eq!(l[199], 0.0);
        // frame 200 on the timeline = asset frame 100
        assert!((l[200] - 0.1).abs() < 1e-6);
        assert!((r[599] - 0.499).abs() < 1e-6);
        assert_eq!(l[600], 0.0);
        // rendering in chunks gives the same result
        let (mut l2, mut r2) = (vec![0.0; 1000], vec![0.0; 1000]);
        for k in (0..1000).step_by(64) {
            let n = 64.min(1000 - k);
            rd.render(0, k as u64, &mut l2[k..k + n], &mut r2[k..k + n]);
        }
        assert_eq!(l, l2);
    }

    #[test]
    fn missing_asset_is_silent_and_reported() {
        let d = tempfile::tempdir().unwrap();
        let (mut p, t, aid) = setup(d.path());
        edit::add_clip(&mut p, t, aid, 0).unwrap();
        std::fs::remove_file(d.path().join("audio/a.wav")).unwrap();
        let set = build_sources(&p, resolver(d.path().to_path_buf()));
        assert_eq!(set.missing_assets, vec![aid]);
        let mut rd = SourceReader::new(Arc::new(set));
        let (mut l, mut r) = (vec![1.0; 100], vec![1.0; 100]);
        rd.render(0, 0, &mut l, &mut r);
        assert!(l.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn comp_switches_takes_with_crossfade() {
        let d = tempfile::tempdir().unwrap();
        let (mut p, t, aid) = setup(d.path());
        // second asset: constant -1
        crate::wav::write_wav_f32(&d.path().join("audio/b.wav"), 48_000, 1, &vec![-1.0; 1000]).unwrap();
        let b = Asset {
            id: new_id(),
            file: "audio/b.wav".into(),
            original_name: "b".into(),
            kind: AssetKind::Recording,
            sample_rate: 48_000,
            channels: 1,
            frames: 1000,
            created_at: Utc::now(),
        };
        let bid = b.id;
        p.assets.push(b);
        let c1 = edit::add_clip(&mut p, t, aid, 0).unwrap();
        let c2 = edit::add_clip(&mut p, t, bid, 0).unwrap();
        let g = edit::register_take(&mut p, t, c2).unwrap().unwrap();
        edit::set_comp(&mut p, t, g, 500, 1000, c1).unwrap();
        let set = Arc::new(build_sources(&p, resolver(d.path().to_path_buf())));
        let mut rd = SourceReader::new(set);
        let (mut l, mut r) = (vec![0.0; 1000], vec![0.0; 1000]);
        rd.render(0, 0, &mut l, &mut r);
        assert!((l[100] + 1.0).abs() < 1e-6, "take 2 before the switch");
        assert!((l[900] - 0.9).abs() < 1e-6, "take 1 after the switch");
        // inside the crossfade both contribute, no hard step
        let mid = l[500];
        assert!(mid > -1.0 && mid < 0.5, "{mid}");
        for w in l[430..570].windows(2) {
            assert!((w[1] - w[0]).abs() < 0.05, "smooth crossfade");
        }
    }
}
