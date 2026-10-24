//! Loading VST3 modules and reading their factories.

use std::path::{Path, PathBuf};

use uta_plugin_protocol::PluginInfo;
use vst3::Steinberg::{IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2, TUID};
use vst3::ComPtr;

use crate::com::read_cstr;

pub const AUDIO_EFFECT_CLASS: &str = "Audio Module Class";

/// Resolve the binary inside a `.vst3` bundle (or accept a single-file module).
pub fn module_binary(path: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if !path.is_dir() {
        return Err(format!("{} does not exist", path.display()));
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (arch_dir, ext) = if cfg!(target_os = "windows") {
        ("x86_64-win", "vst3")
    } else if cfg!(target_os = "macos") {
        return Err("macOS bundles are not supported by this build".into());
    } else {
        ("x86_64-linux", "so")
    };
    let dir = path.join("Contents").join(arch_dir);
    let preferred = dir.join(format!("{stem}.{ext}"));
    if preferred.is_file() {
        return Ok(preferred);
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == ext).unwrap_or(false) && p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(format!("no 64-bit {} binary found in {} (is this a 32-bit or wrong-platform plugin?)", ext, dir.display()))
}

pub struct Module {
    /// Released in `Drop` before the library is unloaded.
    factory: Option<ComPtr<IPluginFactory>>,
    lib: Option<libloading::Library>,
}

type GetFactory = unsafe extern "system" fn() -> *mut IPluginFactory;

impl Module {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bin = module_binary(path)?;
        // SAFETY: loading a plugin runs its initialisers; this is exactly what
        // the helper process exists to isolate.
        let lib = unsafe { open_library(&bin)? };
        unsafe {
            #[cfg(target_os = "windows")]
            if let Ok(init) = lib.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") {
                if !init() {
                    return Err("the plugin's InitDll() failed".into());
                }
            }
            #[cfg(target_os = "linux")]
            if let Ok(entry) = lib.get::<unsafe extern "system" fn(*mut std::ffi::c_void) -> bool>(b"ModuleEntry\0") {
                if !entry(std::ptr::null_mut()) {
                    return Err("the plugin's ModuleEntry() failed".into());
                }
            }
            let get: libloading::Symbol<GetFactory> = lib
                .get(b"GetPluginFactory\0")
                .map_err(|_| "not a VST3 plugin (no GetPluginFactory entry point)".to_string())?;
            let raw = get();
            let factory = ComPtr::from_raw(raw).ok_or_else(|| "GetPluginFactory returned null".to_string())?;
            Ok(Self { factory: Some(factory), lib: Some(lib) })
        }
    }

    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("factory is present until drop")
    }

    pub fn classes(&self) -> Vec<(TUID, PluginInfo)> {
        let mut out = Vec::new();
        unsafe {
            let mut vendor = String::new();
            let mut finfo: vst3::Steinberg::PFactoryInfo = std::mem::zeroed();
            if self.factory().getFactoryInfo(&mut finfo) == vst3::Steinberg::kResultOk {
                vendor = read_cstr(&finfo.vendor);
            }
            let f2 = self.factory().cast::<IPluginFactory2>();
            let n = self.factory().countClasses();
            for i in 0..n.max(0) {
                let mut info = PluginInfo {
                    uid: String::new(),
                    name: String::new(),
                    vendor: vendor.clone(),
                    version: String::new(),
                    category: String::new(),
                    sub_categories: String::new(),
                    is_instrument: false,
                    audio_inputs: 0,
                    audio_outputs: 0,
                };
                let cid: TUID;
                let mut got2 = false;
                if let Some(f2) = f2.as_ref() {
                    let mut c2: PClassInfo2 = std::mem::zeroed();
                    if f2.getClassInfo2(i, &mut c2) == vst3::Steinberg::kResultOk {
                        info.name = read_cstr(&c2.name);
                        info.category = read_cstr(&c2.category);
                        info.sub_categories = read_cstr(&c2.subCategories);
                        let v = read_cstr(&c2.vendor);
                        if !v.is_empty() {
                            info.vendor = v;
                        }
                        info.version = read_cstr(&c2.version);
                        cid = c2.cid;
                        got2 = true;
                    } else {
                        cid = [0; 16];
                    }
                } else {
                    cid = [0; 16];
                }
                let cid = if got2 {
                    cid
                } else {
                    let mut c: PClassInfo = std::mem::zeroed();
                    if self.factory().getClassInfo(i, &mut c) != vst3::Steinberg::kResultOk {
                        continue;
                    }
                    info.name = read_cstr(&c.name);
                    info.category = read_cstr(&c.category);
                    c.cid
                };
                info.uid = tuid_to_hex(&cid);
                info.is_instrument = info.sub_categories.contains("Instrument");
                out.push((cid, info));
            }
        }
        out
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // release the factory first, then call the exit hook and unload
        drop(self.factory.take());
        if let Some(lib) = self.lib.take() {
            unsafe {
                #[cfg(target_os = "windows")]
                if let Ok(exit) = lib.get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0") {
                    exit();
                }
                #[cfg(target_os = "linux")]
                if let Ok(exit) = lib.get::<unsafe extern "system" fn() -> bool>(b"ModuleExit\0") {
                    exit();
                }
            }
            drop(lib);
        }
    }
}

unsafe fn open_library(bin: &Path) -> Result<libloading::Library, String> {
    libloading::Library::new(bin).map_err(|e| format!("could not load {}: {e}", bin.display()))
}

pub fn tuid_to_hex(t: &TUID) -> String {
    t.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

pub fn hex_to_tuid(s: &str) -> Option<TUID> {
    if s.len() != 32 {
        return None;
    }
    let mut t: TUID = [0; 16];
    for (i, item) in t.iter_mut().enumerate() {
        *item = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()? as std::ffi::c_char;
    }
    Some(t)
}
