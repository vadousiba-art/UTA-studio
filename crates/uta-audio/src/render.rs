//! Offline rendering (mixdown / export).
//!
//! Renders the project through the same [`Graph`] code used for live
//! playback, reading clips straight from disk. Output is streamed to the WAV
//! writer chunk by chunk (with streaming sample-rate conversion when the
//! export rate differs), so memory use stays flat regardless of song length.
//! After writing, the file is re-opened and decoded to verify it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use serde::{Deserialize, Serialize};

use crate::error::{AudioError, Result};
use crate::graph::{Graph, TrackFeed, MAX_BLOCK};
use crate::source::{SourceReader, SourceSet};
use crate::wav::{wav_info, BitDepth, WavFileWriter};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOptions {
    /// Timeline range in project frames.
    pub start: u64,
    pub end: u64,
    /// Extra seconds rendered after `end` for reverb/delay tails.
    pub tail_seconds: f32,
    /// Output sample rate (None = project rate).
    pub sample_rate: Option<u32>,
    pub depth: BitDepth,
    /// 1 = mono (L+R)/2, 2 = stereo.
    pub channels: u16,
    pub dither: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderReport {
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f64,
    pub peak_db: f32,
    /// Samples at or above full scale *before* conversion (they clip in
    /// integer formats).
    pub clipped_samples: u64,
    /// The written file was re-opened and its header and length matched.
    pub verified: bool,
    pub warnings: Vec<String>,
}

struct FileFeed<'a> {
    reader: &'a mut SourceReader,
    position: u64,
}

impl TrackFeed for FileFeed<'_> {
    fn fill(&mut self, index: usize, _id: u128, l: &mut [f32], r: &mut [f32]) {
        self.reader.render(index, self.position, l, r);
    }
}

/// Streaming FFT resampler wrapper for interleaved stereo/mono.
struct StreamResampler {
    rs: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    to_trim: usize,
}

impl StreamResampler {
    fn new(from: u32, to: u32, channels: usize) -> Result<Self> {
        let rs = Fft::<f32>::new(from as usize, to as usize, 1024, channels, FixedSync::Input)
            .map_err(|e| AudioError::Other(format!("resampler setup failed: {e}")))?;
        let to_trim = rs.output_delay();
        let out = vec![0.0; rs.output_frames_max() * channels];
        Ok(Self { rs, channels, pending: Vec::new(), out, to_trim })
    }

    /// Feed interleaved samples; returns produced interleaved samples.
    fn push(&mut self, input: &[f32], flush: bool) -> Result<Vec<f32>> {
        self.pending.extend_from_slice(input);
        if flush {
            // Push silence through so the resampler's internal delay line is
            // emptied; the caller trims the output to the exact length.
            let pad = (self.rs.input_frames_max() * 2 + self.rs.output_delay() * 2) * self.channels;
            self.pending.extend(std::iter::repeat_n(0.0, pad));
        }
        let mut produced = Vec::new();
        loop {
            let need = self.rs.input_frames_next();
            let have = self.pending.len() / self.channels;
            if have < need && !(flush && have > 0) {
                break;
            }
            let frames = have.min(need);
            let inp = InterleavedSlice::new(&self.pending[..frames * self.channels], self.channels, frames)
                .map_err(|e| AudioError::Other(e.to_string()))?;
            let out_frames = self.out.len() / self.channels;
            let mut outp = InterleavedSlice::new_mut(&mut self.out, self.channels, out_frames)
                .map_err(|e| AudioError::Other(e.to_string()))?;
            let idx = Indexing {
                input_offset: 0,
                output_offset: 0,
                partial_len: if frames < need { Some(frames) } else { None },
                active_channels_mask: None,
            };
            let (used, wrote) = self
                .rs
                .process_into_buffer(&inp, &mut outp, Some(&idx))
                .map_err(|e| AudioError::Other(format!("resampling failed: {e}")))?;
            let used = used.min(frames);
            self.pending.drain(..used * self.channels);
            let mut start = 0;
            if self.to_trim > 0 {
                let t = self.to_trim.min(wrote);
                self.to_trim -= t;
                start = t;
            }
            produced.extend_from_slice(&self.out[start * self.channels..wrote * self.channels]);
            if frames < need {
                break;
            }
        }
        Ok(produced)
    }
}

/// Render the project to `path`. `progress` receives 0..1 and returns
/// `false` to cancel (the partial file is deleted).
pub fn render_to_wav(
    mut graph: Graph,
    sources: Arc<SourceSet>,
    project_rate: u32,
    opts: &RenderOptions,
    path: &Path,
    mut progress: impl FnMut(f32) -> bool,
) -> Result<RenderReport> {
    if opts.end <= opts.start {
        return Err(AudioError::Other("the export range is empty".into()));
    }
    if !(1..=2).contains(&opts.channels) {
        return Err(AudioError::Other("export must be mono or stereo".into()));
    }
    let out_rate = opts.sample_rate.unwrap_or(project_rate);
    if !(8_000..=192_000).contains(&out_rate) {
        return Err(AudioError::Other(format!("unsupported export sample rate {out_rate}")));
    }
    let ch = opts.channels as usize;
    let tail = (opts.tail_seconds.max(0.0) * project_rate as f32) as u64;
    let total = opts.end - opts.start + tail;
    let tmp_path = path.with_extension("wav.part");
    let mut writer = WavFileWriter::create(&tmp_path, out_rate, opts.channels, opts.depth, opts.dither)?;
    let mut resampler = if out_rate != project_rate {
        Some(StreamResampler::new(project_rate, out_rate, ch)?)
    } else {
        None
    };
    let mut reader = SourceReader::new(sources);
    graph.reset();
    let mut l = vec![0.0f32; MAX_BLOCK];
    let mut r = vec![0.0f32; MAX_BLOCK];
    let mut inter = Vec::with_capacity(MAX_BLOCK * 2);
    let mut peak = 0.0f32;
    let mut clipped = 0u64;
    let mut done = 0u64;
    let mut warnings = Vec::new();

    let cleanup = |w: WavFileWriter| {
        let _ = w.finalize();
        let _ = std::fs::remove_file(&tmp_path);
    };

    // plugin latency delays the output: render that much extra and drop it
    // from the start so the file lines up with the timeline
    let skip = graph.latency() as u64;
    let mut processed = 0u64;
    while done < total {
        let n = (total + skip - processed).min(MAX_BLOCK as u64) as usize;
        let mut feed = FileFeed { reader: &mut reader, position: opts.start + processed };
        graph.process(n, &mut feed, &[], &mut l, &mut r, None);
        let from = skip.saturating_sub(processed).min(n as u64) as usize;
        processed += n as u64;
        inter.clear();
        for k in from..n {
            let (a, b) = (l[k], r[k]);
            for v in [a, b] {
                let m = v.abs();
                peak = peak.max(m);
                if m >= 1.0 {
                    clipped += 1;
                }
            }
            if ch == 1 {
                inter.push(0.5 * (a + b));
            } else {
                inter.push(a);
                inter.push(b);
            }
        }
        let chunk = match resampler.as_mut() {
            Some(rs) => rs.push(&inter, false)?,
            None => inter.clone(),
        };
        if let Err(e) = writer.write(&chunk) {
            cleanup(writer);
            return Err(AudioError::write(path, e));
        }
        done += (n - from) as u64;
        if processed % (MAX_BLOCK as u64 * 64) < MAX_BLOCK as u64 && !progress(done as f32 / total as f32) {
            cleanup(writer);
            return Err(AudioError::Cancelled);
        }
    }
    if let Some(rs) = resampler.as_mut() {
        let rest = rs.push(&[], true)?;
        // trim to the exact expected length
        let expected = (total as f64 * out_rate as f64 / project_rate as f64).round() as u64;
        let have = writer.frames();
        let keep = (expected.saturating_sub(have) as usize * ch).min(rest.len());
        writer.write(&rest[..keep]).map_err(|e| AudioError::write(path, e))?;
    }
    for (_, e) in reader.errors.drain(..) {
        warnings.push(format!("a clip could not be read: {e}"));
    }
    let frames = writer.finalize().map_err(|e| AudioError::write(path, e))?;
    std::fs::rename(&tmp_path, path).map_err(|e| AudioError::write(path, e))?;
    progress(1.0);

    // verify
    let verified = match wav_info(path) {
        Ok(i) => i.frames == frames && i.sample_rate == out_rate && i.channels == opts.channels,
        Err(e) => {
            warnings.push(format!("verification failed: {e}"));
            false
        }
    };
    if clipped > 0 {
        warnings.push(format!(
            "{clipped} samples reached full scale; lower the master or track levels to avoid distortion"
        ));
    }
    Ok(RenderReport {
        path: path.to_path_buf(),
        frames,
        sample_rate: out_rate,
        channels: opts.channels,
        duration_seconds: frames as f64 / out_rate as f64,
        peak_db: uta_dsp::gain_to_db(peak),
        clipped_samples: clipped,
        verified,
        warnings,
    })
}
