//! Host-side COM objects handed to plugins: host context, component
//! handler, memory stream and parameter-change queues.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, IHostApplication, IHostApplicationTrait, IParamValueQueue, IParamValueQueueTrait,
    IParameterChanges, IParameterChangesTrait, ParamID, ParamValue, String128,
};
use vst3::Steinberg::{int32, int64, kInvalidArgument, kNotImplemented, kResultFalse, kResultOk, tresult, IBStream, IBStreamTrait, TUID};
use vst3::{Class, ComWrapper};

pub fn write_wstr(src: &str, dst: &mut [u16]) {
    let mut n = 0;
    for (s, d) in src.encode_utf16().zip(dst.iter_mut()) {
        *d = s;
        n += 1;
    }
    if n < dst.len() {
        dst[n] = 0;
    } else if let Some(l) = dst.last_mut() {
        *l = 0;
    }
}

pub fn read_wstr(src: &[u16]) -> String {
    let end = src.iter().position(|c| *c == 0).unwrap_or(src.len());
    String::from_utf16_lossy(&src[..end])
}

pub fn read_cstr(src: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = src.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ------------------------------------------------------------- host app

pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        write_wstr("UTA Vocal Lab", &mut *name);
        kResultOk
    }

    unsafe fn createInstance(&self, _cid: *mut TUID, _iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        // IMessage/IAttributeList creation is not offered; plugins must cope.
        if !obj.is_null() {
            *obj = std::ptr::null_mut();
        }
        kNotImplemented
    }
}

// ------------------------------------------------------ component handler

/// Receives parameter edits a plugin makes on its own (e.g. linked
/// parameters). Edits are collected so the host can report them.
#[derive(Default)]
pub struct ComponentHandler {
    pub edits: RefCell<Vec<(ParamID, ParamValue)>>,
    pub restart_flags: Cell<i32>,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        let mut e = self.edits.borrow_mut();
        if e.len() < 4096 {
            e.push((id, value));
        }
        kResultOk
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.restart_flags.set(self.restart_flags.get() | flags);
        kResultOk
    }
}

// ---------------------------------------------------------------- stream

#[derive(Default)]
pub struct MemoryStream {
    pub data: RefCell<Vec<u8>>,
    pub pos: Cell<usize>,
}

impl MemoryStream {
    pub fn with_data(d: Vec<u8>) -> Self {
        Self { data: RefCell::new(d), pos: Cell::new(0) }
    }
}

impl Class for MemoryStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemoryStream {
    unsafe fn read(&self, buffer: *mut c_void, num_bytes: int32, num_read: *mut int32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let d = self.data.borrow();
        let p = self.pos.get().min(d.len());
        let n = (num_bytes as usize).min(d.len() - p);
        std::ptr::copy_nonoverlapping(d.as_ptr().add(p), buffer as *mut u8, n);
        self.pos.set(p + n);
        if !num_read.is_null() {
            *num_read = n as int32;
        }
        if n == 0 && num_bytes > 0 {
            kResultFalse
        } else {
            kResultOk
        }
    }

    unsafe fn write(&self, buffer: *mut c_void, num_bytes: int32, num_written: *mut int32) -> tresult {
        if buffer.is_null() || num_bytes < 0 {
            return kInvalidArgument;
        }
        let mut d = self.data.borrow_mut();
        let p = self.pos.get();
        let n = num_bytes as usize;
        if d.len() < p + n {
            d.resize(p + n, 0);
        }
        std::ptr::copy_nonoverlapping(buffer as *const u8, d.as_mut_ptr().add(p), n);
        self.pos.set(p + n);
        if !num_written.is_null() {
            *num_written = num_bytes;
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let len = self.data.borrow().len() as i64;
        let base = match mode {
            0 => 0,
            1 => self.pos.get() as i64,
            2 => len,
            _ => return kInvalidArgument,
        };
        let np = (base + pos).max(0);
        self.pos.set(np as usize);
        if !result.is_null() {
            *result = np;
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        *pos = self.pos.get() as i64;
        kResultOk
    }
}

// ------------------------------------------------------ parameter changes

pub struct ParamQueue {
    pub id: Cell<ParamID>,
    pub points: RefCell<Vec<(i32, f64)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }
    unsafe fn getPointCount(&self) -> int32 {
        self.points.borrow().len() as int32
    }
    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        match self.points.borrow().get(index as usize) {
            Some(&(o, v)) => {
                if !offset.is_null() {
                    *offset = o;
                }
                if !value.is_null() {
                    *value = v;
                }
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let mut p = self.points.borrow_mut();
        p.push((offset, value));
        if !index.is_null() {
            *index = (p.len() - 1) as int32;
        }
        kResultOk
    }
}

/// A fixed pool of queues, reused every block (no per-block allocation of
/// COM objects).
pub struct ParamChanges {
    pub queues: Vec<ComWrapper<ParamQueue>>,
    pub used: Cell<usize>,
}

impl ParamChanges {
    pub fn new(capacity: usize) -> Self {
        Self {
            queues: (0..capacity)
                .map(|_| ComWrapper::new(ParamQueue { id: Cell::new(0), points: RefCell::new(Vec::with_capacity(4)) }))
                .collect(),
            used: Cell::new(0),
        }
    }

    pub fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.points.borrow_mut().clear();
        }
        self.used.set(0);
    }

    pub fn set(&self, id: ParamID, value: f64) {
        let n = self.used.get();
        if let Some(q) = self.queues[..n].iter().find(|q| q.id.get() == id) {
            let mut p = q.points.borrow_mut();
            p.clear();
            p.push((0, value));
            return;
        }
        if n < self.queues.len() {
            let q = &self.queues[n];
            q.id.set(id);
            q.points.borrow_mut().clear();
            q.points.borrow_mut().push((0, value));
            self.used.set(n + 1);
        }
    }

    pub fn drain(&self) -> Vec<(u32, f64)> {
        let mut v = Vec::new();
        for q in &self.queues[..self.used.get()] {
            if let Some(&(_, val)) = q.points.borrow().last() {
                v.push((q.id.get(), val));
            }
        }
        self.clear();
        v
    }
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used.get() as int32
    }
    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.used.get() {
            return std::ptr::null_mut();
        }
        self.queues[index as usize]
            .as_com_ref::<IParamValueQueue>()
            .map(|r| r.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }
    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        let n = self.used.get();
        let pid = *id;
        let i = match self.queues[..n].iter().position(|q| q.id.get() == pid) {
            Some(i) => i,
            None if n < self.queues.len() => {
                self.queues[n].id.set(pid);
                self.queues[n].points.borrow_mut().clear();
                self.used.set(n + 1);
                n
            }
            None => return std::ptr::null_mut(),
        };
        if !index.is_null() {
            *index = i as int32;
        }
        self.queues[i]
            .as_com_ref::<IParamValueQueue>()
            .map(|r| r.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }
}
