//! State shared between the audio callback and control threads, using only
//! atomics (no locks on the real-time side).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub const MAX_TRACKS: usize = 64;
pub const MAX_INPUT_CHANNELS: usize = 16;

#[inline]
fn f32_load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

/// Lock-free peak/RMS meter cell. The audio thread raises the peak with
/// `fetch_max` (valid because non-negative f32 bit patterns order like
/// integers); the reader takes and resets it.
#[derive(Default)]
pub struct MeterCell {
    peak: AtomicU32,
    rms: AtomicU32,
    clip: AtomicBool,
}

impl MeterCell {
    #[inline]
    pub fn update(&self, peak: f32, rms: f32) {
        let p = if peak.is_finite() { peak.max(0.0) } else { 1e9 };
        self.peak.fetch_max(p.to_bits(), Ordering::Relaxed);
        self.rms.store(rms.max(0.0).to_bits(), Ordering::Relaxed);
        if p >= uta_dsp::meter::CLIP_THRESHOLD {
            self.clip.store(true, Ordering::Relaxed);
        }
    }

    /// Read and reset the peak; RMS is the last block value. The clip flag is
    /// sticky until [`MeterCell::reset_clip`].
    pub fn take(&self) -> MeterValue {
        MeterValue {
            peak: f32::from_bits(self.peak.swap(0, Ordering::Relaxed)),
            rms: f32_load(&self.rms),
            clipped: self.clip.load(Ordering::Relaxed),
        }
    }

    pub fn reset_clip(&self) {
        self.clip.store(false, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.peak.store(0, Ordering::Relaxed);
        self.rms.store(0, Ordering::Relaxed);
        self.clip.store(false, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterValue {
    pub peak: f32,
    pub rms: f32,
    pub clipped: bool,
}

#[derive(Default)]
pub struct StereoMeter {
    pub left: MeterCell,
    pub right: MeterCell,
}

impl StereoMeter {
    #[inline]
    pub fn update(&self, l: &[f32], r: &[f32]) {
        let (pl, rl) = peak_rms(l);
        let (pr, rr) = peak_rms(r);
        self.left.update(pl, rl);
        self.right.update(pr, rr);
    }
    pub fn take(&self) -> [MeterValue; 2] {
        [self.left.take(), self.right.take()]
    }
    pub fn reset_clip(&self) {
        self.left.reset_clip();
        self.right.reset_clip();
    }
    pub fn clear(&self) {
        self.left.clear();
        self.right.clear();
    }
}

#[inline]
pub fn peak_rms(s: &[f32]) -> (f32, f32) {
    if s.is_empty() {
        return (0.0, 0.0);
    }
    let mut peak = 0.0f32;
    let mut sum = 0.0f32;
    for &x in s {
        peak = peak.max(x.abs());
        sum += x * x;
    }
    (peak, (sum / s.len() as f32).sqrt())
}

/// Everything the UI needs to poll, written by the audio thread.
pub struct SharedState {
    pub sample_rate: AtomicU32,
    /// Current transport position (frames, project timeline).
    pub position: AtomicU64,
    pub playing: AtomicBool,
    pub recording: AtomicBool,
    /// Timeline position where the current recording started.
    pub record_start: AtomicU64,
    /// Frames pushed to the recorder in the current recording.
    pub record_frames: AtomicU64,
    /// Frames the recorder ring could not accept (should stay 0).
    pub record_dropped: AtomicU64,
    /// Input ring had less data than the output needed (input clock slower,
    /// or input stream stalled).
    pub input_underruns: AtomicU64,
    /// Input ring overflowed (output stalled / clock drift).
    pub input_overflows: AtomicU64,
    /// A track's playback stream ran dry (disk too slow).
    pub stream_underruns: AtomicU64,
    /// Number of output callbacks processed.
    pub callbacks: AtomicU64,
    /// Fraction of the block time spent processing, as f32 bits.
    pub dsp_load: AtomicU32,
    /// Generation of the playback streams currently in use.
    pub stream_generation: AtomicU64,
    /// Preview playback position (frames into the preview file).
    pub preview_position: AtomicU64,
    pub preview_playing: AtomicBool,
    /// Recorded frames captured before the current preview started (for
    /// aligning a take with the backing), `u64::MAX` when the preview was
    /// started while not recording.
    pub preview_start_record_frame: AtomicU64,
    /// Set by the device error callback.
    pub device_lost: AtomicBool,
    /// Device xruns reported by the backend.
    pub device_xruns: AtomicU64,
    /// Latest latency estimates from stream timestamps (microseconds).
    pub input_latency_us: AtomicU64,
    pub output_latency_us: AtomicU64,
    pub track_meters: Vec<StereoMeter>,
    pub master_meter: StereoMeter,
    pub input_meters: Vec<MeterCell>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            sample_rate: AtomicU32::new(48_000),
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            record_start: AtomicU64::new(0),
            record_frames: AtomicU64::new(0),
            record_dropped: AtomicU64::new(0),
            input_underruns: AtomicU64::new(0),
            input_overflows: AtomicU64::new(0),
            stream_underruns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            dsp_load: AtomicU32::new(0),
            stream_generation: AtomicU64::new(0),
            preview_position: AtomicU64::new(0),
            preview_playing: AtomicBool::new(false),
            preview_start_record_frame: AtomicU64::new(u64::MAX),
            device_lost: AtomicBool::new(false),
            device_xruns: AtomicU64::new(0),
            input_latency_us: AtomicU64::new(0),
            output_latency_us: AtomicU64::new(0),
            track_meters: (0..MAX_TRACKS).map(|_| StereoMeter::default()).collect(),
            master_meter: StereoMeter::default(),
            input_meters: (0..MAX_INPUT_CHANNELS).map(|_| MeterCell::default()).collect(),
        }
    }

    pub fn dsp_load(&self) -> f32 {
        f32_load(&self.dsp_load)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_cell_peak_hold_and_clip() {
        let m = MeterCell::default();
        m.update(0.5, 0.1);
        m.update(0.3, 0.2);
        let v = m.take();
        assert_eq!(v.peak, 0.5);
        assert_eq!(v.rms, 0.2);
        assert!(!v.clipped);
        assert_eq!(m.take().peak, 0.0);
        m.update(1.2, 0.5);
        assert!(m.take().clipped);
        assert!(m.take().clipped, "sticky");
        m.reset_clip();
        assert!(!m.take().clipped);
    }
}
