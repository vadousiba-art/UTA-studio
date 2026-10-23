//! Recording-quality assessment: decides whether a take supports grading.

use serde::{Deserialize, Serialize};

use crate::dynamics::DynamicsSummary;
use crate::pitch::PitchFrame;
use crate::stats::{mean, median, percentile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityLevel {
    /// Measurements are reliable.
    Good,
    /// Measurements are usable but less certain; feedback says so.
    Fair,
    /// The take cannot support grading; ask for a cleaner recording.
    Poor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityIssue {
    TooShort,
    NothingSung,
    TooQuiet,
    Clipping,
    Noisy,
    LowConfidence,
    BackgroundUnknown,
}

impl QualityIssue {
    pub fn message(self) -> &'static str {
        match self {
            QualityIssue::TooShort => "The recording is too short to analyse.",
            QualityIssue::NothingSung => "Very little singing was detected.",
            QualityIssue::TooQuiet => "The input level is very low — move closer or raise the input gain.",
            QualityIssue::Clipping => "The input clipped (distorted). Lower the input gain or step back slightly.",
            QualityIssue::Noisy => "Background noise is high compared to your voice.",
            QualityIssue::LowConfidence => "Pitch could not be measured confidently for much of the take.",
            QualityIssue::BackgroundUnknown => "No silence was recorded, so background noise could not be estimated.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingQuality {
    pub level: QualityLevel,
    pub issues: Vec<QualityIssue>,
    /// 10th-percentile frame level (dBFS) — the background estimate.
    pub noise_floor_db: Option<f32>,
    /// Median voiced-frame level (dBFS).
    pub signal_db: Option<f32>,
    pub snr_db: Option<f32>,
    pub voiced_ratio: f32,
    pub mean_confidence: Option<f32>,
}

pub fn assess_quality(duration: f32, frames: &[PitchFrame], dynamics: &DynamicsSummary) -> RecordingQuality {
    let mut issues = Vec::new();
    let mut level = QualityLevel::Good;
    let mut worsen = |l: QualityLevel, issue: QualityIssue, issues: &mut Vec<QualityIssue>| {
        level = level.max(l);
        issues.push(issue);
    };

    let n = frames.len().max(1);
    let voiced: Vec<&PitchFrame> = frames.iter().filter(|f| f.is_voiced()).collect();
    let unvoiced_levels: Vec<f32> = frames.iter().filter(|f| !f.is_voiced()).map(|f| f.rms_db).collect();
    let voiced_ratio = voiced.len() as f32 / n as f32;
    let conf: Vec<f32> = voiced.iter().map(|f| f.confidence).collect();
    let mean_conf = mean(&conf);
    let signal_db = median(&voiced.iter().map(|f| f.rms_db).collect::<Vec<_>>());
    // Need at least ~150 ms of unvoiced material for a background estimate.
    let noise_floor_db = if unvoiced_levels.len() >= 15 {
        percentile(&unvoiced_levels, 10.0)
    } else {
        None
    };
    let snr_db = match (signal_db, noise_floor_db) {
        (Some(s), Some(nf)) => Some(s - nf),
        _ => None,
    };

    if duration < 0.4 {
        worsen(QualityLevel::Poor, QualityIssue::TooShort, &mut issues);
    }
    if voiced.len() < 20 || voiced_ratio < 0.1 {
        worsen(QualityLevel::Poor, QualityIssue::NothingSung, &mut issues);
    }
    if let Some(s) = signal_db {
        if s < -45.0 {
            worsen(QualityLevel::Poor, QualityIssue::TooQuiet, &mut issues);
        } else if s < -38.0 {
            worsen(QualityLevel::Fair, QualityIssue::TooQuiet, &mut issues);
        }
    }
    if dynamics.clip_ratio > 0.01 {
        worsen(QualityLevel::Poor, QualityIssue::Clipping, &mut issues);
    } else if dynamics.clipped_samples > 0 {
        worsen(QualityLevel::Fair, QualityIssue::Clipping, &mut issues);
    }
    match snr_db {
        Some(snr) if snr < 12.0 => worsen(QualityLevel::Poor, QualityIssue::Noisy, &mut issues),
        Some(snr) if snr < 22.0 => worsen(QualityLevel::Fair, QualityIssue::Noisy, &mut issues),
        None if !voiced.is_empty() => issues.push(QualityIssue::BackgroundUnknown),
        _ => {}
    }
    if let Some(c) = mean_conf {
        if c < 0.68 {
            worsen(QualityLevel::Poor, QualityIssue::LowConfidence, &mut issues);
        } else if c < 0.78 {
            worsen(QualityLevel::Fair, QualityIssue::LowConfidence, &mut issues);
        }
    }

    RecordingQuality {
        level,
        issues,
        noise_floor_db,
        signal_db,
        snr_db,
        voiced_ratio,
        mean_confidence: mean_conf,
    }
}
