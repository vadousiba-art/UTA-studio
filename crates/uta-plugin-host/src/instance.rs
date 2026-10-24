//! One hosted VST3 audio-effect instance.

use std::path::Path;
use std::sync::Arc;

use uta_plugin_protocol::{ParamDesc, ParamValue as PValue, PluginInfo};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, BusDirections_, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait, IConnectionPoint,
    IConnectionPointTrait, IEditController, IEditControllerTrait, IHostApplication, IParameterChanges, MediaTypes_, ParameterInfo,
    ProcessContext, ProcessData, ProcessSetup, SpeakerArrangement, String128,
};
use vst3::Steinberg::{kResultOk, kResultTrue, FUnknown, IBStream, IPluginBaseTrait, IPluginFactoryTrait};
use vst3::{ComPtr, ComWrapper, Interface};

use crate::com::{read_wstr, ComponentHandler, HostApp, MemoryStream, ParamChanges};
use crate::module::{hex_to_tuid, Module, AUDIO_EFFECT_CLASS};

const KSTEREO: SpeakerArrangement = 3;
const PROCESS_REALTIME: i32 = 0;
const PROCESS_OFFLINE: i32 = 2;
const SAMPLE32: i32 = 0;
const FLAG_CAN_AUTOMATE: i32 = 1;
const FLAG_READ_ONLY: i32 = 2;
const FLAG_BYPASS: i32 = 1 << 16;

fn channels(arr: SpeakerArrangement) -> usize {
    arr.count_ones() as usize
}

struct Bus {
    channels: usize,
    buffers: Vec<Vec<f32>>,
    ptrs: Vec<*mut f32>,
}

impl Bus {
    fn new(ch: usize, max_block: usize) -> Self {
        let mut buffers: Vec<Vec<f32>> = (0..ch).map(|_| vec![0.0; max_block]).collect();
        let ptrs = buffers.iter_mut().map(|b| b.as_mut_ptr()).collect();
        Self { channels: ch, buffers, ptrs }
    }
}

pub struct Instance {
    pub info: PluginInfo,
    pub params: Vec<ParamDesc>,
    pub latency: u32,
    pub tail: u32,
    pub warnings: Vec<String>,
    // COM objects (dropped explicitly in `Drop`, before the module)
    component: Option<ComPtr<IComponent>>,
    processor: Option<ComPtr<IAudioProcessor>>,
    controller: Option<ComPtr<IEditController>>,
    separate_controller: bool,
    connections: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    host: ComWrapper<HostApp>,
    handler: ComWrapper<ComponentHandler>,
    changes: ComWrapper<ParamChanges>,
    out_changes: ComWrapper<ParamChanges>,
    inputs: Vec<Bus>,
    outputs: Vec<Bus>,
    in_abb: Vec<AudioBusBuffers>,
    out_abb: Vec<AudioBusBuffers>,
    context: ProcessContext,
    max_block: usize,
    offline: bool,
    active: bool,
    _module: Arc<Module>,
}

fn host_unknown(h: &ComWrapper<HostApp>) -> *mut FUnknown {
    h.to_com_ptr::<IHostApplication>()
        .map(|p| p.into_raw() as *mut FUnknown)
        .unwrap_or(std::ptr::null_mut())
}

fn stream(data: Vec<u8>) -> ComWrapper<MemoryStream> {
    ComWrapper::new(MemoryStream::with_data(data))
}

fn stream_ptr(s: &ComWrapper<MemoryStream>) -> *mut IBStream {
    s.as_com_ref::<IBStream>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut())
}

impl Instance {
    pub fn load(
        module: Arc<Module>,
        path: &Path,
        uid: &str,
        sample_rate: f64,
        max_block: u32,
        offline: bool,
        component_state: Option<Vec<u8>>,
        controller_state: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        let _ = path;
        let cid = hex_to_tuid(uid).ok_or_else(|| format!("invalid plugin id {uid}"))?;
        let classes = module.classes();
        let (_, info) = classes
            .into_iter()
            .find(|(c, i)| *c == cid && i.category == AUDIO_EFFECT_CLASS)
            .ok_or_else(|| "this plugin no longer contains the expected audio effect (was it updated or replaced?)".to_string())?;
        if info.is_instrument {
            return Err("instrument plugins are not supported in this version (effects only)".into());
        }
        let max_block = (max_block as usize).clamp(16, uta_plugin_protocol::MAX_BLOCK as usize);
        let host = ComWrapper::new(HostApp);
        let handler = ComWrapper::new(ComponentHandler::default());
        let mut warnings = Vec::new();
        unsafe {
            let factory = module.factory();
            let mut obj: *mut std::ffi::c_void = std::ptr::null_mut();
            let r = factory.createInstance(cid.as_ptr(), IComponent::IID.as_ptr() as *const std::ffi::c_char, &mut obj);
            let component = (if r == kResultOk { ComPtr::from_raw(obj as *mut IComponent) } else { None })
                .ok_or_else(|| "the plugin refused to create its audio component".to_string())?;
            if component.initialize(host_unknown(&host)) != kResultOk {
                return Err("the plugin's component failed to initialise".into());
            }
            let processor = component
                .cast::<IAudioProcessor>()
                .ok_or_else(|| "the plugin has no audio processor".to_string())?;

            // controller: same object, or a separate class
            let mut separate = false;
            let controller = match component.cast::<IEditController>() {
                Some(c) => Some(c),
                None => {
                    let mut ccid: vst3::Steinberg::TUID = [0; 16];
                    if component.getControllerClassId(&mut ccid) == kResultOk && ccid != [0; 16] {
                        let mut cobj: *mut std::ffi::c_void = std::ptr::null_mut();
                        let r = factory.createInstance(ccid.as_ptr(), IEditController::IID.as_ptr() as *const std::ffi::c_char, &mut cobj);
                        match if r == kResultOk { ComPtr::from_raw(cobj as *mut IEditController) } else { None } {
                            Some(c) => {
                                if c.initialize(host_unknown(&host)) == kResultOk {
                                    separate = true;
                                    Some(c)
                                } else {
                                    warnings.push("the plugin's controller failed to initialise; parameters are unavailable".into());
                                    None
                                }
                            }
                            None => {
                                warnings.push("the plugin's controller could not be created; parameters are unavailable".into());
                                None
                            }
                        }
                    } else {
                        None
                    }
                }
            };
            let mut connections = None;
            if let Some(c) = controller.as_ref() {
                if let Some(h) = handler.to_com_ptr::<IComponentHandler>() {
                    c.setComponentHandler(h.as_ptr());
                }
                if separate {
                    if let (Some(a), Some(b)) = (component.cast::<IConnectionPoint>(), c.cast::<IConnectionPoint>()) {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                        connections = Some((a, b));
                    }
                }
            }

            // buses
            let audio = MediaTypes_::kAudio as i32;
            let n_in = component.getBusCount(audio, BusDirections_::kInput as i32).max(0) as usize;
            let n_out = component.getBusCount(audio, BusDirections_::kOutput as i32).max(0) as usize;
            if n_out == 0 {
                return Err("the plugin has no audio output".into());
            }
            if n_in == 0 {
                return Err("the plugin has no audio input, so it can't be used as an effect".into());
            }
            let mut in_arr: Vec<SpeakerArrangement> = (0..n_in)
                .map(|i| {
                    let mut a: SpeakerArrangement = 0;
                    processor.getBusArrangement(BusDirections_::kInput as i32, i as i32, &mut a);
                    a
                })
                .collect();
            let mut out_arr: Vec<SpeakerArrangement> = (0..n_out)
                .map(|i| {
                    let mut a: SpeakerArrangement = 0;
                    processor.getBusArrangement(BusDirections_::kOutput as i32, i as i32, &mut a);
                    a
                })
                .collect();
            in_arr[0] = KSTEREO;
            out_arr[0] = KSTEREO;
            if processor.setBusArrangements(in_arr.as_mut_ptr(), n_in as i32, out_arr.as_mut_ptr(), n_out as i32) != kResultTrue {
                // keep whatever the plugin prefers (e.g. mono) and adapt
                for (i, a) in in_arr.iter_mut().enumerate() {
                    processor.getBusArrangement(BusDirections_::kInput as i32, i as i32, a);
                }
                for (i, a) in out_arr.iter_mut().enumerate() {
                    processor.getBusArrangement(BusDirections_::kOutput as i32, i as i32, a);
                }
                warnings.push(format!("the plugin runs with {} input / {} output channels", channels(in_arr[0]), channels(out_arr[0])));
            }
            if channels(in_arr[0]) == 0 || channels(out_arr[0]) == 0 {
                return Err("the plugin reported an unusable channel layout".into());
            }
            for i in 0..n_in {
                component.activateBus(audio, BusDirections_::kInput as i32, i as i32, (i == 0) as u8);
            }
            for i in 0..n_out {
                component.activateBus(audio, BusDirections_::kOutput as i32, i as i32, (i == 0) as u8);
            }
            if processor.canProcessSampleSize(SAMPLE32) != kResultOk {
                return Err("the plugin can't process 32-bit audio".into());
            }
            let mut setup = ProcessSetup {
                processMode: if offline { PROCESS_OFFLINE } else { PROCESS_REALTIME },
                symbolicSampleSize: SAMPLE32,
                maxSamplesPerBlock: max_block as i32,
                sampleRate: sample_rate,
            };
            if processor.setupProcessing(&mut setup) != kResultOk {
                return Err(format!("the plugin rejected {sample_rate} Hz / {max_block}-sample blocks"));
            }

            let mut inst = Self {
                info,
                params: Vec::new(),
                latency: 0,
                tail: 0,
                warnings,
                component: Some(component),
                processor: Some(processor),
                controller,
                separate_controller: separate,
                connections,
                host,
                handler,
                changes: ComWrapper::new(ParamChanges::new(256)),
                out_changes: ComWrapper::new(ParamChanges::new(64)),
                inputs: in_arr.iter().map(|a| Bus::new(channels(*a), max_block)).collect(),
                outputs: out_arr.iter().map(|a| Bus::new(channels(*a), max_block)).collect(),
                in_abb: Vec::new(),
                out_abb: Vec::new(),
                context: std::mem::zeroed(),
                max_block,
                offline,
                active: false,
                _module: module,
            };
            // restore state before activation
            if let Some(cs) = component_state {
                if let Err(e) = inst.set_state(&cs, controller_state.as_deref()) {
                    inst.warnings.push(format!("saved plugin state could not be restored: {e}"));
                }
            } else if inst.separate_controller {
                // sync controller with the component's default state
                if let Ok(cs) = inst.component_state() {
                    let s = stream(cs);
                    if let Some(c) = inst.controller.as_ref() {
                        c.setComponentState(stream_ptr(&s));
                    }
                }
            }
            inst.read_params();
            let comp = inst.component.as_ref().expect("component");
            if comp.setActive(1) != kResultOk {
                return Err("the plugin failed to activate".into());
            }
            inst.active = true;
            let proc_ = inst.processor.as_ref().expect("processor");
            proc_.setProcessing(1);
            inst.latency = proc_.getLatencySamples();
            inst.tail = proc_.getTailSamples();
            inst.context.sampleRate = sample_rate;
            inst.context.tempo = 120.0;
            inst.context.timeSigNumerator = 4;
            inst.context.timeSigDenominator = 4;
            Ok(inst)
        }
    }

    fn read_params(&mut self) {
        self.params.clear();
        let Some(c) = self.controller.as_ref() else { return };
        unsafe {
            let n = c.getParameterCount().clamp(0, 10_000);
            for i in 0..n {
                let mut pi: ParameterInfo = std::mem::zeroed();
                if c.getParameterInfo(i, &mut pi) != kResultOk {
                    continue;
                }
                self.params.push(ParamDesc {
                    id: pi.id,
                    title: read_wstr(&pi.title),
                    units: read_wstr(&pi.units),
                    default_normalized: pi.defaultNormalizedValue,
                    step_count: pi.stepCount,
                    can_automate: pi.flags & FLAG_CAN_AUTOMATE != 0,
                    read_only: pi.flags & FLAG_READ_ONLY != 0,
                    is_bypass: pi.flags & FLAG_BYPASS != 0,
                });
            }
        }
    }

    pub fn param_values(&self) -> Vec<PValue> {
        let Some(c) = self.controller.as_ref() else { return Vec::new() };
        self.params
            .iter()
            .map(|p| unsafe {
                let v = c.getParamNormalized(p.id);
                let mut s: String128 = [0; 128];
                let display = if c.getParamStringByValue(p.id, v, &mut s) == kResultOk { read_wstr(&s) } else { format!("{v:.3}") };
                PValue { id: p.id, value: v, display }
            })
            .collect()
    }

    /// Queue parameter changes for the next block and update the controller.
    pub fn set_params(&self, values: &[(u32, f64)]) {
        for &(id, v) in values {
            let v = v.clamp(0.0, 1.0);
            self.changes.set(id, v);
            if let Some(c) = self.controller.as_ref() {
                unsafe {
                    c.setParamNormalized(id, v);
                }
            }
        }
    }

    pub fn component_state(&self) -> Result<Vec<u8>, String> {
        let s = stream(Vec::new());
        let r = unsafe { self.component.as_ref().expect("component").getState(stream_ptr(&s)) };
        if r != kResultOk {
            return Err("the plugin could not save its state".into());
        }
        let d = s.data.borrow().clone();
        Ok(d)
    }

    pub fn controller_state(&self) -> Option<Vec<u8>> {
        if !self.separate_controller {
            return None;
        }
        let c = self.controller.as_ref()?;
        let s = stream(Vec::new());
        if unsafe { c.getState(stream_ptr(&s)) } != kResultOk {
            return None;
        }
        let d = s.data.borrow().clone();
        Some(d)
    }

    pub fn set_state(&mut self, component: &[u8], controller: Option<&[u8]>) -> Result<(), String> {
        unsafe {
            let s = stream(component.to_vec());
            if self.component.as_ref().expect("component").setState(stream_ptr(&s)) != kResultOk {
                return Err("the plugin rejected the saved state".into());
            }
            if let Some(c) = self.controller.as_ref() {
                let s2 = stream(component.to_vec());
                c.setComponentState(stream_ptr(&s2));
                if let (Some(cs), true) = (controller, self.separate_controller) {
                    let s3 = stream(cs.to_vec());
                    c.setState(stream_ptr(&s3));
                }
            }
        }
        Ok(())
    }

    /// Process one block of stereo audio in place.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), String> {
        let n = left.len().min(right.len());
        let mut done = 0;
        while done < n {
            let frames = (n - done).min(self.max_block);
            self.process_block(&mut left[done..done + frames], &mut right[done..done + frames])?;
            done += frames;
        }
        Ok(())
    }

    /// Deliver queued parameter changes to the processor without audio
    /// (a VST3 "parameter flush": a process call with zero samples), so a
    /// following state read reflects them.
    pub fn flush_params(&mut self) -> Result<(), String> {
        if self.changes.used.get() == 0 {
            return Ok(());
        }
        self.process_block(&mut [], &mut [])
    }

    fn process_block(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), String> {
        let frames = left.len();
        // feed input bus 0 (stereo, or mono downmix), silence other buses
        for (bi, bus) in self.inputs.iter_mut().enumerate() {
            for (ci, buf) in bus.buffers.iter_mut().enumerate() {
                let b = &mut buf[..frames];
                if bi == 0 {
                    match (bus.channels, ci) {
                        (1, _) => {
                            for k in 0..frames {
                                b[k] = 0.5 * (left[k] + right[k]);
                            }
                        }
                        (_, 0) => b.copy_from_slice(left),
                        (_, 1) => b.copy_from_slice(right),
                        _ => b.fill(0.0),
                    }
                } else {
                    b.fill(0.0);
                }
            }
        }
        for bus in self.outputs.iter_mut() {
            for buf in bus.buffers.iter_mut() {
                buf[..frames].fill(0.0);
            }
        }
        self.in_abb.clear();
        for bus in self.inputs.iter_mut() {
            let mut abb: AudioBusBuffers = unsafe { std::mem::zeroed() };
            abb.numChannels = bus.channels as i32;
            abb.__field0.channelBuffers32 = bus.ptrs.as_mut_ptr();
            self.in_abb.push(abb);
        }
        self.out_abb.clear();
        for bus in self.outputs.iter_mut() {
            let mut abb: AudioBusBuffers = unsafe { std::mem::zeroed() };
            abb.numChannels = bus.channels as i32;
            abb.__field0.channelBuffers32 = bus.ptrs.as_mut_ptr();
            self.out_abb.push(abb);
        }
        let changes: *mut IParameterChanges = self.changes.as_com_ref::<IParameterChanges>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut());
        let out_changes: *mut IParameterChanges = self.out_changes.as_com_ref::<IParameterChanges>().map(|r| r.as_ptr()).unwrap_or(std::ptr::null_mut());
        self.context.state = 0;
        let mut data = ProcessData {
            processMode: if self.offline { PROCESS_OFFLINE } else { PROCESS_REALTIME },
            symbolicSampleSize: SAMPLE32,
            numSamples: frames as i32,
            numInputs: self.in_abb.len() as i32,
            numOutputs: self.out_abb.len() as i32,
            inputs: self.in_abb.as_mut_ptr(),
            outputs: self.out_abb.as_mut_ptr(),
            inputParameterChanges: changes,
            outputParameterChanges: out_changes,
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut self.context,
        };
        let r = unsafe { self.processor.as_ref().expect("processor").process(&mut data) };
        self.changes.clear();
        // plugin-initiated parameter changes keep the controller in sync
        let outs = self.out_changes.drain();
        if let Some(c) = self.controller.as_ref() {
            for (id, v) in outs {
                unsafe {
                    c.setParamNormalized(id, v);
                }
            }
        }
        self.context.continousTimeSamples += frames as i64;
        if r != kResultOk {
            return Err(format!("the plugin's process() returned error {r}"));
        }
        let out = &self.outputs[0];
        if out.channels == 1 {
            left.copy_from_slice(&out.buffers[0][..frames]);
            right.copy_from_slice(&out.buffers[0][..frames]);
        } else {
            left.copy_from_slice(&out.buffers[0][..frames]);
            right.copy_from_slice(&out.buffers[1][..frames]);
        }
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            if self.active {
                if let Some(p) = self.processor.as_ref() {
                    p.setProcessing(0);
                }
                if let Some(c) = self.component.as_ref() {
                    c.setActive(0);
                }
            }
            if let Some((a, b)) = self.connections.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = self.controller.take() {
                c.setComponentHandler(std::ptr::null_mut());
                if self.separate_controller {
                    c.terminate();
                }
            }
            self.processor = None;
            if let Some(c) = self.component.take() {
                c.terminate();
            }
        }
        let _ = (&self.host, &self.handler);
    }
}
