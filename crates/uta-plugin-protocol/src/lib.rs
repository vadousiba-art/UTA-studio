//! Wire protocol between UTA Vocal Lab and `uta-plugin-host`, the helper
//! process that loads third-party VST3 plugins.
//!
//! Plugins run in a separate process so that a crashing or hanging plugin
//! cannot take down the app or corrupt a project. The helper talks over its
//! stdin/stdout using length-prefixed frames:
//!
//! ```text
//! u32 LE  payload length (excluding these 4 bytes)
//! u8      frame kind (1 = JSON message, 2 = audio request, 3 = audio reply)
//! ...     payload
//! ```
//!
//! Audio frames are binary to keep per-block overhead low:
//!
//! * request: `u32 frames, u32 n_params, n_params × (u32 id, f64 value),
//!   frames × f32 left, frames × f32 right`
//! * reply: `u32 frames, frames × f32 left, frames × f32 right`
//!
//! Stdout is reserved for frames; the helper logs to stderr only.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
/// Refuse absurd frames (corrupt stream) instead of allocating them.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Largest audio block the helper accepts.
pub const MAX_BLOCK: u32 = 8192;

pub const KIND_JSON: u8 = 1;
pub const KIND_AUDIO_REQ: u8 = 2;
pub const KIND_AUDIO_REPLY: u8 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// Class id as 32 hex digits.
    pub uid: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// VST3 class category (e.g. "Audio Module Class").
    pub category: String,
    /// Sub-categories (e.g. "Fx|EQ").
    pub sub_categories: String,
    /// True for instruments (not hosted as effects in this version).
    pub is_instrument: bool,
    pub audio_inputs: u32,
    pub audio_outputs: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDesc {
    pub id: u32,
    pub title: String,
    pub units: String,
    pub default_normalized: f64,
    /// 0 = continuous, 1 = toggle, n = n+1 discrete steps.
    pub step_count: i32,
    pub can_automate: bool,
    pub read_only: bool,
    pub is_bypass: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamValue {
    pub id: u32,
    pub value: f64,
    /// Plugin's own text for the value (e.g. "-6.0 dB").
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    Hello,
    /// Load the module at `path` and describe its classes. Nothing is
    /// instantiated beyond the factory.
    #[serde(rename_all = "camelCase")]
    Scan { path: String },
    /// Instantiate one audio effect class and prepare it for processing.
    #[serde(rename_all = "camelCase")]
    Load {
        path: String,
        uid: String,
        sample_rate: f64,
        max_block: u32,
        offline: bool,
        /// Previously saved state (hex encoded), applied before activation.
        component_state: Option<String>,
        controller_state: Option<String>,
    },
    GetParams,
    #[serde(rename_all = "camelCase")]
    SetParams { values: Vec<(u32, f64)> },
    GetState,
    #[serde(rename_all = "camelCase")]
    SetState { component_state: String, controller_state: Option<String> },
    /// Test hook: abort the helper process (crash-recovery tests).
    CrashForTesting,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Response {
    #[serde(rename_all = "camelCase")]
    Hello { version: u32, pid: u32 },
    #[serde(rename_all = "camelCase")]
    Scanned { plugins: Vec<PluginInfo> },
    #[serde(rename_all = "camelCase")]
    Loaded {
        info: PluginInfo,
        params: Vec<ParamDesc>,
        latency_samples: u32,
        tail_samples: u32,
        /// Notes about things that didn't fully work (e.g. state restore).
        warnings: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Params { values: Vec<ParamValue> },
    #[serde(rename_all = "camelCase")]
    State { component_state: String, controller_state: Option<String> },
    Ok,
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// Hex encoding for opaque state blobs (dependency-free, deterministic).
pub fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let v = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    s.as_bytes().chunks(2).map(|p| Some(v(p[0])? << 4 | v(p[1])?)).collect()
}

/// One received frame.
#[derive(Debug)]
pub enum Frame {
    Json(Vec<u8>),
    AudioRequest { frames: u32, params: Vec<(u32, f64)>, left: Vec<f32>, right: Vec<f32> },
    AudioReply { left: Vec<f32>, right: Vec<f32> },
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

pub fn write_json<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(|e| bad(&e.to_string()))?;
    w.write_all(&((body.len() + 1) as u32).to_le_bytes())?;
    w.write_all(&[KIND_JSON])?;
    w.write_all(&body)?;
    w.flush()
}

/// Write an audio request. `buf` is a reusable scratch buffer.
pub fn write_audio_request<W: Write>(w: &mut W, params: &[(u32, f64)], left: &[f32], right: &[f32], buf: &mut Vec<u8>) -> io::Result<()> {
    let frames = left.len().min(right.len());
    buf.clear();
    buf.push(KIND_AUDIO_REQ);
    buf.extend_from_slice(&(frames as u32).to_le_bytes());
    buf.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for (id, v) in params {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for s in &left[..frames] {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    for s in &right[..frames] {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    w.write_all(&(buf.len() as u32).to_le_bytes())?;
    w.write_all(buf)?;
    w.flush()
}

pub fn write_audio_reply<W: Write>(w: &mut W, left: &[f32], right: &[f32], buf: &mut Vec<u8>) -> io::Result<()> {
    let frames = left.len().min(right.len());
    buf.clear();
    buf.push(KIND_AUDIO_REPLY);
    buf.extend_from_slice(&(frames as u32).to_le_bytes());
    for s in &left[..frames] {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    for s in &right[..frames] {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    w.write_all(&(buf.len() as u32).to_le_bytes())?;
    w.write_all(buf)?;
    w.flush()
}

fn read_f32s(p: &[u8], n: usize) -> Vec<f32> {
    p[..n * 4].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Read one frame. Returns `Ok(None)` on clean EOF.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad("invalid frame length"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let kind = body[0];
    let p = &body[1..];
    match kind {
        KIND_JSON => Ok(Some(Frame::Json(p.to_vec()))),
        KIND_AUDIO_REQ => {
            if p.len() < 8 {
                return Err(bad("short audio request"));
            }
            let frames = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let np = u32::from_le_bytes([p[4], p[5], p[6], p[7]]) as usize;
            if frames > MAX_BLOCK || np > 100_000 {
                return Err(bad("audio request too large"));
            }
            let need = 8 + np * 12 + frames as usize * 8;
            if p.len() < need {
                return Err(bad("truncated audio request"));
            }
            let mut params = Vec::with_capacity(np);
            let mut o = 8;
            for _ in 0..np {
                let id = u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
                let mut vb = [0u8; 8];
                vb.copy_from_slice(&p[o + 4..o + 12]);
                params.push((id, f64::from_le_bytes(vb)));
                o += 12;
            }
            let n = frames as usize;
            let left = read_f32s(&p[o..], n);
            let right = read_f32s(&p[o + n * 4..], n);
            Ok(Some(Frame::AudioRequest { frames, params, left, right }))
        }
        KIND_AUDIO_REPLY => {
            if p.len() < 4 {
                return Err(bad("short audio reply"));
            }
            let n = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize;
            if p.len() < 4 + n * 8 {
                return Err(bad("truncated audio reply"));
            }
            Ok(Some(Frame::AudioReply { left: read_f32s(&p[4..], n), right: read_f32s(&p[4 + n * 4..], n) }))
        }
        _ => Err(bad("unknown frame kind")),
    }
}

pub fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> io::Result<T> {
    serde_json::from_slice(bytes).map_err(|e| bad(&format!("bad message: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_json(&mut buf, &Request::Scan { path: "C:/x.vst3".into() }).unwrap();
        let mut scratch = Vec::new();
        write_audio_request(&mut buf, &[(3, 0.25)], &[0.1, 0.2], &[-0.1, -0.2], &mut scratch).unwrap();
        write_audio_reply(&mut buf, &[1.0], &[2.0], &mut scratch).unwrap();
        let mut r = &buf[..];
        match read_frame(&mut r).unwrap().unwrap() {
            Frame::Json(j) => assert_eq!(parse_json::<Request>(&j).unwrap(), Request::Scan { path: "C:/x.vst3".into() }),
            _ => panic!(),
        }
        match read_frame(&mut r).unwrap().unwrap() {
            Frame::AudioRequest { frames, params, left, right } => {
                assert_eq!(frames, 2);
                assert_eq!(params, vec![(3, 0.25)]);
                assert_eq!(left, vec![0.1, 0.2]);
                assert_eq!(right, vec![-0.1, -0.2]);
            }
            _ => panic!(),
        }
        match read_frame(&mut r).unwrap().unwrap() {
            Frame::AudioReply { left, right } => assert_eq!((left, right), (vec![1.0], vec![2.0])),
            _ => panic!(),
        }
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn rejects_garbage() {
        let mut r = &[0xff, 0xff, 0xff, 0xff, 1][..];
        assert!(read_frame(&mut r).is_err());
        let mut r = &[2, 0, 0, 0, 9, 0][..];
        assert!(read_frame(&mut r).is_err());
    }

    #[test]
    fn hex() {
        let b = vec![0u8, 1, 127, 128, 255];
        assert_eq!(from_hex(&to_hex(&b)).unwrap(), b);
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }
}
