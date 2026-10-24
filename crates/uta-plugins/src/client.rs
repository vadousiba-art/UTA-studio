//! Spawning the helper process and talking to it with timeouts.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use parking_lot::Mutex;
use uta_plugin_protocol::{read_frame, write_audio_request, write_json, Frame, Request, Response, PROTOCOL_VERSION};

/// File name of the helper executable.
pub const HELPER_NAME: &str = "uta-plugin-host";

/// How long ordinary requests may take before the helper is considered hung.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Loading a plugin can be slow (large sample libraries, licence checks).
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(45);
/// One audio block. Far longer than any healthy plugin needs; on expiry the
/// helper is killed and the slot falls back to the dry signal.
pub const AUDIO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum ClientError {
    #[error("the plugin helper could not be started: {0}")]
    Spawn(String),
    #[error("the plugin did not respond within {0} seconds and was stopped")]
    Timeout(u64),
    #[error("the plugin helper stopped unexpectedly{0}")]
    Crashed(String),
    #[error("plugin helper protocol error: {0}")]
    Protocol(String),
    #[error("{0}")]
    Plugin(String),
}

/// Find the helper executable: `UTA_PLUGIN_HOST`, then next to the running
/// executable (installed app, `tauri dev`), then one level up (test
/// binaries live in `target/<profile>/deps`).
pub fn locate_helper() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("UTA_PLUGIN_HOST") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let name = format!("{HELPER_NAME}{}", std::env::consts::EXE_SUFFIX);
    let mut dir = exe.parent();
    for _ in 0..2 {
        let d = dir?;
        let c = d.join(&name);
        if c.is_file() {
            return Some(c);
        }
        dir = d.parent();
    }
    None
}

type StderrTail = Arc<Mutex<VecDeque<String>>>;

/// A connection to one helper process.
pub struct HelperClient {
    child: Child,
    writer: BufWriter<ChildStdin>,
    frames: Receiver<std::io::Result<Frame>>,
    scratch: Vec<u8>,
    stderr: StderrTail,
    dead: Option<ClientError>,
    pub pid: u32,
}

impl HelperClient {
    pub fn spawn(helper: &Path) -> Result<Self, ClientError> {
        let mut cmd = Command::new(helper);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| ClientError::Spawn(format!("{}: {e}", helper.display())))?;
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| ClientError::Spawn("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| ClientError::Spawn("no stdout".into()))?;
        let stderr = child.stderr.take();
        let (tx, rx) = crossbeam_channel::bounded(4);
        std::thread::Builder::new()
            .name(format!("plugin-helper-{pid}-rx"))
            .spawn(move || {
                let mut r = BufReader::new(stdout);
                loop {
                    match read_frame(&mut r) {
                        Ok(Some(f)) => {
                            if tx.send(Ok(f)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = tx.send(Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "pipe closed")));
                            break;
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
            })
            .map_err(|e| ClientError::Spawn(e.to_string()))?;
        let tail: StderrTail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(err) = stderr {
            let tail = tail.clone();
            let _ = std::thread::Builder::new().name(format!("plugin-helper-{pid}-err")).spawn(move || {
                for line in BufReader::new(err).lines() {
                    let Ok(line) = line else { break };
                    tracing::warn!("plugin helper {pid}: {line}");
                    let mut t = tail.lock();
                    if t.len() >= 20 {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            });
        }
        let mut c = Self { child, writer: BufWriter::new(stdin), frames: rx, scratch: Vec::new(), stderr: tail, dead: None, pid };
        match c.request(&Request::Hello, REQUEST_TIMEOUT)? {
            Response::Hello { version, .. } if version == PROTOCOL_VERSION => Ok(c),
            Response::Hello { version, .. } => Err(ClientError::Spawn(format!(
                "helper speaks protocol {version}, expected {PROTOCOL_VERSION} (mismatched installation?)"
            ))),
            r => Err(ClientError::Protocol(format!("unexpected hello reply {r:?}"))),
        }
    }

    pub fn is_dead(&self) -> bool {
        self.dead.is_some()
    }

    pub fn death(&self) -> Option<&ClientError> {
        self.dead.as_ref()
    }

    /// Last lines the helper wrote to stderr (for crash reports).
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr.lock().iter().cloned().collect()
    }

    fn fail(&mut self, e: ClientError) -> ClientError {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let e = match e {
            ClientError::Crashed(_) => {
                let tail = self.stderr_tail();
                ClientError::Crashed(match tail.last() {
                    Some(l) => format!(" ({l})"),
                    None => String::new(),
                })
            }
            e => e,
        };
        self.dead = Some(e.clone());
        e
    }

    fn check_alive(&self) -> Result<(), ClientError> {
        match &self.dead {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn receive(&mut self, timeout: Duration) -> Result<Frame, ClientError> {
        match self.frames.recv_timeout(timeout) {
            Ok(Ok(f)) => Ok(f),
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => Err(self.fail(ClientError::Crashed(String::new()))),
            Err(RecvTimeoutError::Timeout) => Err(self.fail(ClientError::Timeout(timeout.as_secs().max(1)))),
        }
    }

    /// Send a JSON request and wait for its reply. `Response::Error` from
    /// the helper is turned into [`ClientError::Plugin`] (the helper stays
    /// usable); timeouts and crashes kill the helper.
    pub fn request(&mut self, req: &Request, timeout: Duration) -> Result<Response, ClientError> {
        self.check_alive()?;
        if write_json(&mut self.writer, req).is_err() {
            return Err(self.fail(ClientError::Crashed(String::new())));
        }
        match self.receive(timeout)? {
            Frame::Json(bytes) => match uta_plugin_protocol::parse_json::<Response>(&bytes) {
                Ok(Response::Error { message }) => Err(ClientError::Plugin(message)),
                Ok(r) => Ok(r),
                Err(e) => Err(self.fail(ClientError::Protocol(e.to_string()))),
            },
            other => Err(self.fail(ClientError::Protocol(format!("expected a JSON reply, got {other:?}")))),
        }
    }

    /// Process one block in place.
    pub fn process(&mut self, params: &[(u32, f64)], left: &mut [f32], right: &mut [f32], timeout: Duration) -> Result<(), ClientError> {
        self.check_alive()?;
        if write_audio_request(&mut self.writer, params, left, right, &mut self.scratch).is_err() {
            return Err(self.fail(ClientError::Crashed(String::new())));
        }
        match self.receive(timeout)? {
            Frame::AudioReply { left: l, right: r } if l.len() == left.len() && r.len() == right.len() => {
                left.copy_from_slice(&l);
                right.copy_from_slice(&r);
                Ok(())
            }
            Frame::Json(bytes) => {
                let msg = uta_plugin_protocol::parse_json::<Response>(&bytes)
                    .map(|r| format!("{r:?}"))
                    .unwrap_or_else(|e| e.to_string());
                Err(ClientError::Plugin(msg))
            }
            other => Err(self.fail(ClientError::Protocol(format!("bad audio reply {other:?}")))),
        }
    }

    /// Ask the helper to exit cleanly (the plugin is terminated properly),
    /// killing it if it does not exit within a second.
    pub fn shutdown(&mut self) {
        if self.dead.is_none() {
            let _ = self.request(&Request::Shutdown, Duration::from_secs(2));
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    self.dead = Some(ClientError::Crashed(" (shut down)".into()));
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.dead.is_none() {
            self.dead = Some(ClientError::Crashed(" (shut down)".into()));
        }
    }
}

impl Drop for HelperClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}
