//! WAV reading/writing helpers and offline sample-rate conversion.
//!
//! These functions do file I/O and allocate; they run on worker threads,
//! never on the audio callback.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use serde::{Deserialize, Serialize};

use crate::error::{AudioError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub bits_per_sample: u16,
    pub float: bool,
}

impl WavInfo {
    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 / self.sample_rate.max(1) as f64
    }
}

pub fn wav_info(path: &Path) -> Result<WavInfo> {
    let r = WavReader::open(path).map_err(|e| AudioError::read(path, e))?;
    let s = r.spec();
    Ok(WavInfo {
        sample_rate: s.sample_rate,
        channels: s.channels,
        frames: r.duration() as u64,
        bits_per_sample: s.bits_per_sample,
        float: s.sample_format == SampleFormat::Float,
    })
}

/// Decoded interleaved audio.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Mix down to mono (average of channels).
    pub fn to_mono(&self) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        if ch == 1 {
            return self.samples.clone();
        }
        self.samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    /// One channel (0-based); falls back to mono mix if out of range.
    pub fn channel(&self, index: usize) -> Vec<f32> {
        let ch = self.channels.max(1) as usize;
        if index >= ch {
            return self.to_mono();
        }
        self.samples.iter().skip(index).step_by(ch).copied().collect()
    }
}

fn int_scale(bits: u16) -> f32 {
    1.0 / (1u64 << (bits.clamp(8, 32) - 1)) as f32
}

/// Decode a whole WAV file (PCM 8/16/24/32-bit or 32-bit float) to f32.
/// `max_frames` guards memory use (returns an error if exceeded).
pub fn read_wav(path: &Path, max_frames: Option<u64>) -> Result<AudioBuffer> {
    let mut r = WavReader::open(path).map_err(|e| AudioError::read(path, e))?;
    let spec = r.spec();
    if spec.channels == 0 {
        return Err(AudioError::read(path, "file has no channels"));
    }
    if let Some(max) = max_frames {
        if r.duration() as u64 > max {
            return Err(AudioError::read(
                path,
                format!(
                    "file is too long ({:.0} s); the limit for this operation is {:.0} s",
                    r.duration() as f64 / spec.sample_rate as f64,
                    max as f64 / spec.sample_rate as f64
                ),
            ));
        }
    }
    let samples: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => r
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| AudioError::read(path, e))?,
        SampleFormat::Int => {
            let scale = int_scale(spec.bits_per_sample);
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| AudioError::read(path, e))?
        }
    };
    Ok(AudioBuffer {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

/// Output sample formats for writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

impl BitDepth {
    pub fn spec(self, sample_rate: u32, channels: u16) -> WavSpec {
        match self {
            BitDepth::Int16 => WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
            BitDepth::Int24 => WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 24,
                sample_format: SampleFormat::Int,
            },
            BitDepth::Float32 => WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
        }
    }
}

/// Incremental WAV writer with optional TPDF dither for integer formats.
pub struct WavFileWriter {
    inner: WavWriter<BufWriter<File>>,
    depth: BitDepth,
    dither: bool,
    rng: u32,
    frames: u64,
    channels: u16,
}

impl WavFileWriter {
    pub fn create(path: &Path, sample_rate: u32, channels: u16, depth: BitDepth, dither: bool) -> Result<Self> {
        let inner = WavWriter::create(path, depth.spec(sample_rate, channels)).map_err(|e| AudioError::write(path, e))?;
        Ok(Self {
            inner,
            depth,
            dither,
            rng: 0x1234_5678,
            frames: 0,
            channels,
        })
    }

    #[inline]
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Write interleaved samples.
    pub fn write(&mut self, interleaved: &[f32]) -> std::result::Result<(), hound::Error> {
        match self.depth {
            BitDepth::Float32 => {
                for &s in interleaved {
                    self.inner.write_sample(s)?;
                }
            }
            BitDepth::Int16 | BitDepth::Int24 => {
                let bits = if self.depth == BitDepth::Int16 { 16 } else { 24 };
                let max = ((1i64 << (bits - 1)) - 1) as f32;
                for &s in interleaved {
                    let d = if self.dither {
                        // TPDF dither of ±1 LSB
                        (self.rand() - self.rand()) / max
                    } else {
                        0.0
                    };
                    let v = ((s + d).clamp(-1.0, 1.0) * max).round() as i32;
                    self.inner.write_sample(v)?;
                }
            }
        }
        self.frames += (interleaved.len() / self.channels.max(1) as usize) as u64;
        Ok(())
    }

    /// Flush buffered data and rewrite the header so the file is valid even if
    /// the process stops before `finalize`.
    pub fn flush(&mut self) -> std::result::Result<(), hound::Error> {
        self.inner.flush()
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn finalize(self) -> std::result::Result<u64, hound::Error> {
        let n = self.frames;
        self.inner.finalize()?;
        Ok(n)
    }
}

/// Write a whole interleaved buffer as 32-bit float WAV.
pub fn write_wav_f32(path: &Path, sample_rate: u32, channels: u16, interleaved: &[f32]) -> Result<()> {
    let mut w = WavFileWriter::create(path, sample_rate, channels, BitDepth::Float32, false)?;
    w.write(interleaved).map_err(|e| AudioError::write(path, e))?;
    w.finalize().map_err(|e| AudioError::write(path, e))?;
    Ok(())
}

/// High-quality offline resampling of interleaved audio (FFT-based).
pub fn resample(samples: &[f32], channels: u16, from: u32, to: u32) -> Result<Vec<f32>> {
    if from == to || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let mut rs = Fft::<f32>::new(from as usize, to as usize, 1024, ch, FixedSync::Input)
        .map_err(|e| AudioError::Other(format!("resampler setup failed: {e}")))?;
    let input = InterleavedSlice::new(&samples[..frames * ch], ch, frames)
        .map_err(|e| AudioError::Other(format!("resampler input: {e}")))?;
    let out = rs
        .process_all(&input, frames, None)
        .map_err(|e| AudioError::Other(format!("resampling failed: {e}")))?;
    Ok(out.take_data())
}

/// Random-access reader for streaming playback. Keeps the file open and reads
/// arbitrary frame ranges.
pub struct WavStreamReader {
    reader: WavReader<BufReader<File>>,
    pub info: WavInfo,
    scale: f32,
    position: u64,
}

impl WavStreamReader {
    pub fn open(path: &Path) -> Result<Self> {
        let reader = WavReader::open(path).map_err(|e| AudioError::read(path, e))?;
        let s = reader.spec();
        let info = WavInfo {
            sample_rate: s.sample_rate,
            channels: s.channels,
            frames: reader.duration() as u64,
            bits_per_sample: s.bits_per_sample,
            float: s.sample_format == SampleFormat::Float,
        };
        Ok(Self {
            reader,
            scale: int_scale(s.bits_per_sample),
            info,
            position: 0,
        })
    }

    /// Read up to `frames` frames starting at `start` into `out` (interleaved,
    /// file channel count). Returns frames read (fewer at end of file).
    pub fn read(&mut self, start: u64, frames: usize, out: &mut Vec<f32>) -> std::result::Result<usize, hound::Error> {
        out.clear();
        if start >= self.info.frames {
            return Ok(0);
        }
        if start != self.position {
            self.reader.seek(start as u32)?;
        }
        let n = frames.min((self.info.frames - start) as usize);
        let total = n * self.info.channels as usize;
        if self.info.float {
            for s in self.reader.samples::<f32>().take(total) {
                out.push(s?);
            }
        } else {
            let scale = self.scale;
            for s in self.reader.samples::<i32>().take(total) {
                out.push(s? as f32 * scale);
            }
        }
        let got = out.len() / self.info.channels as usize;
        self.position = start + got as u64;
        Ok(got)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_round_trip_all_depths() {
        let dir = tempfile::tempdir().unwrap();
        let sig: Vec<f32> = (0..4800).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
        for (depth, tol) in [(BitDepth::Float32, 1e-7), (BitDepth::Int24, 1e-5), (BitDepth::Int16, 1e-4)] {
            let p = dir.path().join(format!("{depth:?}.wav"));
            let mut w = WavFileWriter::create(&p, 44_100, 2, depth, false).unwrap();
            w.write(&sig).unwrap();
            assert_eq!(w.finalize().unwrap(), 2400);
            let info = wav_info(&p).unwrap();
            assert_eq!((info.sample_rate, info.channels, info.frames), (44_100, 2, 2400));
            let b = read_wav(&p, None).unwrap();
            assert_eq!(b.samples.len(), sig.len());
            for (a, e) in b.samples.iter().zip(&sig) {
                assert!((a - e).abs() < tol, "{depth:?}");
            }
        }
    }

    #[test]
    fn stream_reader_random_access() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.wav");
        let sig: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        write_wav_f32(&p, 48_000, 1, &sig).unwrap();
        let mut r = WavStreamReader::open(&p).unwrap();
        let mut out = Vec::new();
        assert_eq!(r.read(500, 10, &mut out).unwrap(), 10);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert_eq!(r.read(995, 10, &mut out).unwrap(), 5);
        assert_eq!(r.read(0, 3, &mut out).unwrap(), 3);
        assert_eq!(out[2], 0.002);
        assert_eq!(r.read(2000, 3, &mut out).unwrap(), 0);
    }

    #[test]
    fn resample_preserves_duration_and_pitch() {
        let sr_in = 44_100u32;
        let sig: Vec<f32> = (0..sr_in as usize)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / sr_in as f32).sin())
            .collect();
        let out = resample(&sig, 1, sr_in, 48_000).unwrap();
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        let crossings = out[4800..43_200].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        // 0.8 s at 1 kHz => 800 crossings
        assert!((crossings as i64 - 800).abs() <= 2, "{crossings}");
    }
}
