//! Waveform overview generation.
//!
//! A base resolution of min/max pairs (one per [`BASE_FRAMES`] frames, mono
//! mix) is computed once per file by streaming through it, cached to disk,
//! and decimated on request for any zoom level. Never runs on the audio
//! thread.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{AudioError, Result};
use crate::wav::WavStreamReader;

pub const BASE_FRAMES: usize = 256;
const MAGIC: &[u8; 8] = b"UTAPEAK1";

#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    pub frames: u64,
    pub sample_rate: u32,
    /// `[min, max]` per BASE_FRAMES block.
    pub data: Vec<[f32; 2]>,
}

impl Peaks {
    /// Compute peaks for a WAV file.
    pub fn compute(path: &Path) -> Result<Self> {
        let mut rd = WavStreamReader::open(path)?;
        let ch = rd.info.channels as usize;
        let mut data = Vec::with_capacity((rd.info.frames as usize / BASE_FRAMES) + 1);
        let mut buf = Vec::new();
        let mut pos = 0u64;
        let chunk = BASE_FRAMES * 64;
        loop {
            let got = rd.read(pos, chunk, &mut buf).map_err(|e| AudioError::read(path, e))?;
            if got == 0 {
                break;
            }
            for block in buf.chunks(BASE_FRAMES * ch) {
                let mut mn = f32::MAX;
                let mut mx = f32::MIN;
                for frame in block.chunks(ch) {
                    let v = frame.iter().sum::<f32>() / ch as f32;
                    mn = mn.min(v);
                    mx = mx.max(v);
                }
                data.push([mn, mx]);
            }
            pos += got as u64;
        }
        Ok(Self { frames: rd.info.frames, sample_rate: rd.info.sample_rate, data })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = Vec::with_capacity(24 + self.data.len() * 8);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.frames.to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        for [a, b] in &self.data {
            bytes.extend_from_slice(&a.to_le_bytes());
            bytes.extend_from_slice(&b.to_le_bytes());
        }
        let tmp = path.with_extension("tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| AudioError::write(path, e))?;
        f.write_all(&bytes).map_err(|e| AudioError::write(path, e))?;
        drop(f);
        fs::rename(&tmp, path).map_err(|e| AudioError::write(path, e))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut f = fs::File::open(path).map_err(|e| AudioError::read(path, e))?;
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).map_err(|e| AudioError::read(path, e))?;
        if bytes.len() < 24 || &bytes[..8] != MAGIC {
            return Err(AudioError::read(path, "not a peak cache file"));
        }
        let frames = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        let sample_rate = u32::from_le_bytes(bytes[16..20].try_into().expect("4 bytes"));
        let n = u32::from_le_bytes(bytes[20..24].try_into().expect("4 bytes")) as usize;
        if bytes.len() != 24 + n * 8 {
            return Err(AudioError::read(path, "truncated peak cache"));
        }
        let data = bytes[24..]
            .chunks_exact(8)
            .map(|c| {
                [
                    f32::from_le_bytes(c[..4].try_into().expect("4 bytes")),
                    f32::from_le_bytes(c[4..].try_into().expect("4 bytes")),
                ]
            })
            .collect();
        Ok(Self { frames, sample_rate, data })
    }

    /// Load from cache when it is up to date, otherwise compute and cache.
    pub fn cached(audio: &Path, cache_dir: &Path) -> Result<Self> {
        let name = audio
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "audio".into());
        let cache: PathBuf = cache_dir.join(format!("{name}.peaks"));
        let fresh = match (fs::metadata(audio), fs::metadata(&cache)) {
            (Ok(a), Ok(c)) => match (a.modified(), c.modified()) {
                (Ok(am), Ok(cm)) => cm >= am,
                _ => false,
            },
            _ => false,
        };
        if fresh {
            if let Ok(p) = Self::load(&cache) {
                return Ok(p);
            }
        }
        let p = Self::compute(audio)?;
        if fs::create_dir_all(cache_dir).is_ok() {
            if let Err(e) = p.save(&cache) {
                tracing::warn!("could not cache peaks: {e}");
            }
        }
        Ok(p)
    }

    /// Min/max pairs for `[start, end)` frames in `buckets` columns, as a flat
    /// `[min0, max0, min1, max1, ...]` vector.
    pub fn range(&self, start: u64, end: u64, buckets: usize) -> Vec<f32> {
        let buckets = buckets.clamp(1, 20_000);
        let mut out = Vec::with_capacity(buckets * 2);
        let end = end.min(self.frames).max(start);
        let span = (end - start) as f64;
        for b in 0..buckets {
            let f0 = start as f64 + span * b as f64 / buckets as f64;
            let f1 = start as f64 + span * (b + 1) as f64 / buckets as f64;
            let i0 = (f0 / BASE_FRAMES as f64).floor() as usize;
            let i1 = ((f1 / BASE_FRAMES as f64).ceil() as usize).max(i0 + 1).min(self.data.len());
            let mut mn = 0.0f32;
            let mut mx = 0.0f32;
            if i0 < i1 {
                mn = f32::MAX;
                mx = f32::MIN;
                for p in &self.data[i0..i1] {
                    mn = mn.min(p[0]);
                    mx = mx.max(p[1]);
                }
            }
            out.push(mn);
            out.push(mx);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_compute_cache_and_range() {
        let d = tempfile::tempdir().unwrap();
        let wav = d.path().join("a.wav");
        // 1 s: first half silence, second half full-scale square
        let mut sig = vec![0.0f32; 24_000];
        sig.extend((0..24_000).map(|i| if (i / 100) % 2 == 0 { 0.8 } else { -0.8 }));
        crate::wav::write_wav_f32(&wav, 48_000, 1, &sig).unwrap();
        let p = Peaks::cached(&wav, &d.path().join("cache")).unwrap();
        assert_eq!(p.frames, 48_000);
        assert_eq!(p.data.len(), 48_000 / BASE_FRAMES + 1);
        let r = p.range(0, 48_000, 4);
        assert_eq!(r.len(), 8);
        assert_eq!((r[0], r[1]), (0.0, 0.0));
        assert!((r[6] + 0.8).abs() < 1e-6 && (r[7] - 0.8).abs() < 1e-6);
        // second call loads from cache
        let p2 = Peaks::cached(&wav, &d.path().join("cache")).unwrap();
        assert_eq!(p, p2);
    }
}
