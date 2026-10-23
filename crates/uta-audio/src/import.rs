//! Importing audio into a project and converting recordings.
//!
//! Every project asset is stored as 32-bit float WAV at the project sample
//! rate. The user's original file is never modified: imports are decoded,
//! resampled if needed and written as a new file in the project.

use std::path::Path;

use crate::error::{AudioError, Result};
use crate::wav::{read_wav, resample, write_wav_f32, WavInfo};

/// Longest file accepted for import (protects 8 GB machines): 20 minutes.
pub const MAX_IMPORT_SECONDS: u64 = 20 * 60;

/// Decode `src` (WAV), convert to `rate`, write float WAV to `dest`.
/// Returns the written file's info.
pub fn import_wav(src: &Path, dest: &Path, rate: u32) -> Result<WavInfo> {
    let info = crate::wav::wav_info(src)?;
    if info.channels == 0 || info.channels > 2 {
        return Err(AudioError::read(
            src,
            format!("{} channels; only mono and stereo files can be imported", info.channels),
        ));
    }
    let buf = read_wav(src, Some(MAX_IMPORT_SECONDS * info.sample_rate as u64))?;
    let data = if buf.sample_rate != rate {
        resample(&buf.samples, buf.channels, buf.sample_rate, rate)?
    } else {
        buf.samples
    };
    write_wav_f32(dest, rate, buf.channels, &data)?;
    crate::wav::wav_info(dest)
}

/// Convert a file in place to `rate` (used when the device ran at a
/// different rate than the project). Writes a new file next to it and
/// replaces the original only after the new file is complete.
pub fn convert_rate_in_place(path: &Path, rate: u32) -> Result<WavInfo> {
    let info = crate::wav::wav_info(path)?;
    if info.sample_rate == rate {
        return Ok(info);
    }
    let tmp = path.with_extension("convert.wav");
    import_wav(path, &tmp, rate)?;
    std::fs::rename(&tmp, path).map_err(|e| AudioError::write(path, e))?;
    crate::wav::wav_info(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_resamples_and_keeps_original() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("in.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..44_100 {
            let v = ((i as f32 * 0.03).sin() * 10_000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(-v).unwrap();
        }
        w.finalize().unwrap();
        let before = std::fs::read(&src).unwrap();
        let dest = d.path().join("out.wav");
        let info = import_wav(&src, &dest, 48_000).unwrap();
        assert_eq!(info.sample_rate, 48_000);
        assert_eq!(info.channels, 2);
        assert!((info.frames as i64 - 48_000).abs() <= 2);
        assert!(info.float);
        assert_eq!(std::fs::read(&src).unwrap(), before, "original untouched");
    }

    #[test]
    fn rejects_garbage() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("bad.wav");
        std::fs::write(&src, b"definitely not a wav file").unwrap();
        assert!(import_wav(&src, &d.path().join("o.wav"), 48_000).is_err());
    }
}
