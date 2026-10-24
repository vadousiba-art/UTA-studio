//! Client side of third-party VST3 hosting.
//!
//! Plugins never run inside the application process. Each loaded plugin
//! lives in its own `uta-plugin-host` helper process (see
//! `uta-plugin-host`), reached over stdin/stdout with the frames defined in
//! `uta-plugin-protocol`. This crate:
//!
//! * finds and spawns the helper and talks to it with timeouts
//!   ([`client`]), killing it when it hangs;
//! * discovers `.vst3` bundles and scans each one in a throwaway helper
//!   ([`scan`]), so a plugin that crashes while being scanned only marks
//!   itself as failed;
//! * bridges a live helper into the real-time mixing graph ([`bridge`]) with
//!   a fixed, reported latency and a dry fallback when the helper dies;
//! * keeps one live instance per effect slot and implements the engine's
//!   `PluginFactory` ([`manager`]).

pub mod bridge;
pub mod client;
pub mod manager;
pub mod scan;
pub mod state;
#[doc(hidden)]
pub mod testing;

pub use client::{locate_helper, ClientError, HelperClient};
pub use manager::{InstanceStatus, OfflineFactory, PluginManager, SlotStatus};
pub use scan::{default_folders, discover, scan_bundle, ScanOutcome};
pub use uta_plugin_protocol::{ParamDesc, ParamValue, PluginInfo};
