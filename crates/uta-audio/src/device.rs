//! Audio device enumeration and stream setup through cpal.
//!
//! On Windows the default host is WASAPI (shared mode). ASIO is deliberately
//! not compiled in: the ASIO SDK is GPLv3-licensed, which is incompatible
//! with shipping this app under its current licence (see
//! docs/known-limitations.md).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig, SupportedBufferSize};
use rtrb::{Consumer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::core::{CoreConfig, EngineCore, InputSource};
use crate::error::{AudioError, Result};
use crate::shared::SharedState;

pub const COMMON_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Stable cpal device id string (`host:id`).
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub max_channels: u16,
    /// Common sample rates the device supports.
    pub sample_rates: Vec<u32>,
    pub default_sample_rate: Option<u32>,
    /// Supported buffer size range in frames, if the backend reports it.
    pub buffer_range: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub host: String,
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}

fn describe(dev: &cpal::Device, input: bool, default_id: Option<&str>) -> Option<DeviceInfo> {
    let id = dev.id().ok()?.to_string();
    let name = dev.description().map(|d| d.to_string()).unwrap_or_else(|_| dev.to_string());
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if input {
        dev.supported_input_configs().ok()?.collect()
    } else {
        dev.supported_output_configs().ok()?.collect()
    };
    if ranges.is_empty() {
        return None;
    }
    let max_channels = ranges.iter().map(|r| r.channels()).max().unwrap_or(0);
    let mut rates: Vec<u32> = COMMON_RATES
        .iter()
        .copied()
        .filter(|r| ranges.iter().any(|g| g.contains_rate(*r)))
        .collect();
    rates.dedup();
    let default_cfg = if input {
        dev.default_input_config().ok()
    } else {
        dev.default_output_config().ok()
    };
    let buffer_range = ranges.iter().find_map(|r| match r.buffer_size() {
        SupportedBufferSize::Range { min, max } => Some((*min, *max)),
        SupportedBufferSize::Unknown => None,
    });
    Some(DeviceInfo {
        is_default: default_id == Some(id.as_str()),
        id,
        name,
        max_channels,
        sample_rates: rates,
        default_sample_rate: default_cfg.map(|c| c.sample_rate()),
        buffer_range,
    })
}

/// List input and output devices of the default host.
pub fn list_devices() -> Result<DeviceList> {
    let host = cpal::default_host();
    let def_in = host.default_input_device().and_then(|d| d.id().ok()).map(|i| i.to_string());
    let def_out = host.default_output_device().and_then(|d| d.id().ok()).map(|i| i.to_string());
    let devices: Vec<cpal::Device> = host
        .devices()
        .map_err(|e| AudioError::Device(e.to_string()))?
        .collect();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for d in &devices {
        if d.supports_input() {
            if let Some(i) = describe(d, true, def_in.as_deref()) {
                inputs.push(i);
            }
        }
        if d.supports_output() {
            if let Some(o) = describe(d, false, def_out.as_deref()) {
                outputs.push(o);
            }
        }
    }
    Ok(DeviceList {
        host: host.id().name().to_string(),
        inputs,
        outputs,
    })
}

/// What the user asked for. `None` = system default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSelection {
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
    /// Open an input stream at all (microphone access).
    pub input_enabled: bool,
}

/// What actually opened.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    pub kind: String,
    pub host: String,
    pub output_name: Option<String>,
    pub input_name: Option<String>,
    pub sample_rate: u32,
    pub output_channels: u16,
    pub input_channels: u16,
    pub buffer_frames: Option<u32>,
    /// Warnings about fallbacks that were applied.
    pub notes: Vec<String>,
}

fn find_device(host: &cpal::Host, id: Option<&str>, input: bool) -> Result<cpal::Device> {
    if let Some(id) = id {
        let parsed: cpal::DeviceId = id
            .parse()
            .map_err(|e: cpal::Error| AudioError::Device(format!("invalid device id {id}: {e}")))?;
        if let Some(d) = host.device_by_id(&parsed) {
            return Ok(d);
        }
        return Err(AudioError::NoDevice(format!("device {id} is not connected")));
    }
    let d = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    };
    d.ok_or_else(|| AudioError::NoDevice(if input { "no input device".into() } else { "no output device".into() }))
}

/// Pick a supported config at `rate`, preferring f32.
fn pick_config(dev: &cpal::Device, input: bool, rate: Option<u32>) -> Result<cpal::SupportedStreamConfig> {
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if input {
        dev.supported_input_configs()
            .map_err(|e| AudioError::Device(e.to_string()))?
            .collect()
    } else {
        dev.supported_output_configs()
            .map_err(|e| AudioError::Device(e.to_string()))?
            .collect()
    };
    let default = if input {
        dev.default_input_config()
    } else {
        dev.default_output_config()
    }
    .map_err(|e| AudioError::Device(e.to_string()))?;
    let rate = rate.unwrap_or(default.sample_rate());
    let rank = |f: SampleFormat| match f {
        SampleFormat::F32 => 0,
        SampleFormat::I32 => 1,
        SampleFormat::I16 => 2,
        _ => 9,
    };
    let mut candidates: Vec<cpal::SupportedStreamConfig> = ranges
        .into_iter()
        .filter(|r| rank(r.sample_format()) < 9)
        .filter_map(|r| r.try_with_sample_rate(rate))
        .collect();
    candidates.sort_by_key(|c| (rank(c.sample_format()), if c.channels() >= 2 { 0 } else { 1 }, c.channels()));
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| AudioError::Unsupported(format!("{} Hz is not supported by this device", rate)))
}

fn buffer_size(cfg: &cpal::SupportedStreamConfig, want: Option<u32>) -> BufferSize {
    match (want, cfg.buffer_size()) {
        (Some(w), SupportedBufferSize::Range { min, max }) => BufferSize::Fixed(w.clamp(*min, *max)),
        (Some(w), SupportedBufferSize::Unknown) => BufferSize::Fixed(w),
        (None, _) => BufferSize::Default,
    }
}

fn error_handler(shared: Arc<SharedState>, which: &'static str) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e: cpal::Error| match e.kind() {
        cpal::ErrorKind::Xrun => {
            shared.device_xruns.fetch_add(1, Ordering::Relaxed);
        }
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::DeviceChanged => {
            shared.device_lost.store(true, Ordering::Release);
            tracing::error!("{which} stream lost: {e}");
        }
        cpal::ErrorKind::RealtimeDenied => tracing::warn!("{which}: real-time priority denied: {e}"),
        _ => tracing::warn!("{which} stream error: {e}"),
    }
}

/// Convert between sample formats in the callbacks.
trait SampleConv: cpal::SizedSample + Send + 'static {
    fn to_f32(self) -> f32;
    fn from_f32(v: f32) -> Self;
}
impl SampleConv for f32 {
    fn to_f32(self) -> f32 {
        self
    }
    fn from_f32(v: f32) -> Self {
        v
    }
}
impl SampleConv for i16 {
    fn to_f32(self) -> f32 {
        self as f32 / 32768.0
    }
    fn from_f32(v: f32) -> Self {
        (v.clamp(-1.0, 1.0) * 32767.0) as i16
    }
}
impl SampleConv for i32 {
    fn to_f32(self) -> f32 {
        self as f32 / 2147483648.0
    }
    fn from_f32(v: f32) -> Self {
        (v.clamp(-1.0, 1.0) as f64 * 2147483647.0) as i32
    }
}

/// Largest callback we pre-allocate conversion scratch for.
const MAX_CALLBACK_FRAMES: usize = 16_384;

fn build_output<T: SampleConv>(
    dev: &cpal::Device,
    config: StreamConfig,
    mut core: EngineCore,
    shared: Arc<SharedState>,
) -> Result<cpal::Stream> {
    let ch = config.channels as usize;
    let mut scratch = vec![0.0f32; MAX_CALLBACK_FRAMES * ch];
    let lat_shared = shared.clone();
    let cb = move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
        let ts = info.timestamp();
        let lat = ts.playback.duration_since(ts.callback);
        lat_shared.output_latency_us.store(lat.as_micros() as u64, Ordering::Relaxed);
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>() {
            // SAFETY: T is f32 (checked above), so the slice reinterpretation is exact.
            let out: &mut [f32] = unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut f32, data.len()) };
            core.process(InputSource::Ring, out, ch);
        } else {
            let n = data.len().min(scratch.len());
            core.process(InputSource::Ring, &mut scratch[..n], ch);
            for (d, s) in data.iter_mut().zip(&scratch[..n]) {
                *d = T::from_f32(*s);
            }
        }
    };
    dev.build_output_stream(config, cb, error_handler(shared, "output"), None)
        .map_err(|e| AudioError::Device(format!("could not open output stream: {e}")))
}

fn build_input<T: SampleConv>(
    dev: &cpal::Device,
    config: StreamConfig,
    mut producer: rtrb::Producer<f32>,
    shared: Arc<SharedState>,
) -> Result<cpal::Stream> {
    let lat_shared = shared.clone();
    let cb = move |data: &[T], info: &cpal::InputCallbackInfo| {
        let ts = info.timestamp();
        let lat = ts.callback.duration_since(ts.capture);
        lat_shared.input_latency_us.store(lat.as_micros() as u64, Ordering::Relaxed);
        let n = producer.slots().min(data.len());
        if n < data.len() {
            lat_shared.input_overflows.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut chunk) = producer.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            for (slot, s) in a.iter_mut().chain(b.iter_mut()).zip(data) {
                slot.write(s.to_f32());
            }
            // SAFETY: exactly n slots were written (zip stops at the shorter side, which is n).
            unsafe { chunk.commit_all() };
        }
    };
    dev.build_input_stream(config, cb, error_handler(shared, "input"), None)
        .map_err(|e| AudioError::Device(format!("could not open input stream: {e}")))
}

/// Running cpal streams. Dropping this stops audio.
pub struct CpalStreams {
    _output: cpal::Stream,
    _input: Option<cpal::Stream>,
}

/// Everything needed to construct the engine core once the device
/// configuration is known.
pub type CoreFactory<'a> = dyn FnOnce(CoreConfig, Option<Consumer<f32>>) -> EngineCore + 'a;

/// Open the selected devices and start the streams.
pub fn start_cpal(sel: &DeviceSelection, shared: Arc<SharedState>, make_core: Box<CoreFactory<'_>>) -> Result<(CpalStreams, BackendInfo)> {
    let host = cpal::default_host();
    let mut notes = Vec::new();
    let out_dev = find_device(&host, sel.output.as_deref(), false)?;
    let out_cfg = match pick_config(&out_dev, false, sel.sample_rate) {
        Ok(c) => c,
        Err(e) if sel.sample_rate.is_some() => {
            notes.push(format!("{e}; using the device default rate instead"));
            pick_config(&out_dev, false, None)?
        }
        Err(e) => return Err(e),
    };
    let rate = out_cfg.sample_rate();

    let mut input = None;
    if sel.input_enabled {
        match find_device(&host, sel.input.as_deref(), true).and_then(|d| {
            let c = pick_config(&d, true, Some(rate))?;
            Ok((d, c))
        }) {
            Ok(pair) => input = Some(pair),
            Err(e) => notes.push(format!("microphone unavailable: {e}")),
        }
    }
    let in_channels = input.as_ref().map(|(_, c)| c.channels()).unwrap_or(0);
    let (in_prod, in_cons) = RingBuffer::<f32>::new(((rate as usize) / 2).max(4096) * in_channels.max(1) as usize);

    let core = make_core(
        CoreConfig { sample_rate: rate, input_channels: in_channels },
        if input.is_some() { Some(in_cons) } else { None },
    );
    let mut out_stream_cfg = out_cfg.config();
    out_stream_cfg.buffer_size = buffer_size(&out_cfg, sel.buffer_frames);
    let output = match out_cfg.sample_format() {
        SampleFormat::F32 => build_output::<f32>(&out_dev, out_stream_cfg, core, shared.clone())?,
        SampleFormat::I32 => build_output::<i32>(&out_dev, out_stream_cfg, core, shared.clone())?,
        SampleFormat::I16 => build_output::<i16>(&out_dev, out_stream_cfg, core, shared.clone())?,
        f => return Err(AudioError::Unsupported(format!("output sample format {f}"))),
    };
    let mut input_stream = None;
    let mut input_name = None;
    if let Some((dev, cfg)) = input {
        let mut sc = cfg.config();
        sc.buffer_size = buffer_size(&cfg, sel.buffer_frames);
        let built = match cfg.sample_format() {
            SampleFormat::F32 => build_input::<f32>(&dev, sc, in_prod, shared.clone()),
            SampleFormat::I32 => build_input::<i32>(&dev, sc, in_prod, shared.clone()),
            SampleFormat::I16 => build_input::<i16>(&dev, sc, in_prod, shared.clone()),
            f => Err(AudioError::Unsupported(format!("input sample format {f}"))),
        };
        match built {
            Ok(s) => {
                input_name = Some(dev.description().map(|d| d.to_string()).unwrap_or_else(|_| dev.to_string()));
                input_stream = Some(s);
            }
            Err(e) => notes.push(format!("microphone could not be opened: {e}")),
        }
    }
    output.play().map_err(|e| AudioError::Device(format!("could not start output: {e}")))?;
    if let Some(s) = &input_stream {
        if let Err(e) = s.play() {
            notes.push(format!("microphone could not start: {e}"));
        }
    }
    let buffer_frames = output.buffer_size().ok();
    let info = BackendInfo {
        kind: "device".into(),
        host: host.id().name().to_string(),
        output_name: Some(out_dev.description().map(|d| d.to_string()).unwrap_or_else(|_| out_dev.to_string())),
        input_name,
        sample_rate: rate,
        output_channels: out_cfg.channels(),
        input_channels: if input_stream.is_some() { in_channels } else { 0 },
        buffer_frames,
        notes,
    };
    Ok((CpalStreams { _output: output, _input: input_stream }, info))
}
