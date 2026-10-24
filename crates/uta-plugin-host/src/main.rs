//! `uta-plugin-host`: an isolated helper process that loads one VST3 plugin
//! on behalf of UTA Vocal Lab.
//!
//! Usage: `uta-plugin-host` (frames on stdin/stdout, see
//! `uta-plugin-protocol`). One process hosts at most one plugin instance, so
//! a crash only affects that plugin. Diagnostics go to stderr.

mod com;
mod instance;
mod module;

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;

use uta_plugin_protocol::{from_hex, parse_json, read_frame, to_hex, write_audio_reply, write_json, Frame, Request, Response, PROTOCOL_VERSION};

use crate::instance::Instance;
use crate::module::{Module, AUDIO_EFFECT_CLASS};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    let mut module: Option<Arc<Module>> = None;
    let mut instance: Option<Instance> = None;
    let mut scratch = Vec::new();

    loop {
        let frame = match read_frame(&mut input) {
            Ok(Some(f)) => f,
            Ok(None) => break, // parent closed the pipe
            Err(e) => {
                eprintln!("uta-plugin-host: bad frame: {e}");
                break;
            }
        };
        let reply = match frame {
            Frame::AudioRequest { params, mut left, mut right, .. } => {
                let Some(inst) = instance.as_mut() else {
                    let _ = write_json(&mut output, &Response::Error { message: "no plugin loaded".into() });
                    continue;
                };
                inst.set_params(&params);
                if let Err(e) = inst.process(&mut left, &mut right) {
                    eprintln!("uta-plugin-host: {e}");
                    left.fill(0.0);
                    right.fill(0.0);
                }
                // sanitise: never pass NaN/inf back into the mix
                for s in left.iter_mut().chain(right.iter_mut()) {
                    if !s.is_finite() {
                        *s = 0.0;
                    }
                }
                if write_audio_reply(&mut output, &left, &right, &mut scratch).is_err() {
                    break;
                }
                continue;
            }
            Frame::AudioReply { .. } => Response::Error { message: "unexpected frame".into() },
            Frame::Json(bytes) => match parse_json::<Request>(&bytes) {
                Err(e) => Response::Error { message: e.to_string() },
                Ok(req) => handle(req, &mut module, &mut instance),
            },
        };
        if write_json(&mut output, &reply).is_err() {
            break;
        }
        if SHUTDOWN.with(|s| s.get()) {
            break;
        }
    }
    // drop the instance before the module
    drop(instance);
    drop(module);
}

thread_local! {
    static SHUTDOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn handle(req: Request, module: &mut Option<Arc<Module>>, instance: &mut Option<Instance>) -> Response {
    match req {
        Request::Hello => Response::Hello { version: PROTOCOL_VERSION, pid: std::process::id() },
        Request::Scan { path } => match Module::load(&PathBuf::from(&path)) {
            Ok(m) => {
                let plugins = m
                    .classes()
                    .into_iter()
                    .filter(|(_, i)| i.category == AUDIO_EFFECT_CLASS)
                    .map(|(_, i)| i)
                    .collect();
                *module = Some(Arc::new(m));
                Response::Scanned { plugins }
            }
            Err(e) => Response::Error { message: e },
        },
        Request::Load { path, uid, sample_rate, max_block, offline, component_state, controller_state } => {
            if instance.is_some() {
                return Response::Error { message: "a plugin is already loaded in this helper".into() };
            }
            let m = match module.clone() {
                Some(m) => m,
                None => match Module::load(&PathBuf::from(&path)) {
                    Ok(m) => {
                        let m = Arc::new(m);
                        *module = Some(m.clone());
                        m
                    }
                    Err(e) => return Response::Error { message: e },
                },
            };
            let cs = component_state.as_deref().and_then(from_hex);
            let ctl = controller_state.as_deref().and_then(from_hex);
            match Instance::load(m, &PathBuf::from(&path), &uid, sample_rate, max_block, offline, cs, ctl) {
                Ok(inst) => {
                    let r = Response::Loaded {
                        info: inst.info.clone(),
                        params: inst.params.clone(),
                        latency_samples: inst.latency,
                        tail_samples: inst.tail,
                        warnings: inst.warnings.clone(),
                    };
                    *instance = Some(inst);
                    r
                }
                Err(e) => Response::Error { message: e },
            }
        }
        Request::GetParams => match instance.as_ref() {
            Some(i) => Response::Params { values: i.param_values() },
            None => Response::Error { message: "no plugin loaded".into() },
        },
        Request::SetParams { values } => match instance.as_ref() {
            Some(i) => {
                i.set_params(&values);
                Response::Ok
            }
            None => Response::Error { message: "no plugin loaded".into() },
        },
        Request::GetState => match instance.as_ref() {
            Some(i) => match i.component_state() {
                Ok(c) => Response::State { component_state: to_hex(&c), controller_state: i.controller_state().map(|s| to_hex(&s)) },
                Err(e) => Response::Error { message: e },
            },
            None => Response::Error { message: "no plugin loaded".into() },
        },
        Request::SetState { component_state, controller_state } => match instance.as_mut() {
            Some(i) => {
                let Some(cs) = from_hex(&component_state) else {
                    return Response::Error { message: "state is not valid hex".into() };
                };
                let ctl = controller_state.as_deref().and_then(from_hex);
                match i.set_state(&cs, ctl.as_deref()) {
                    Ok(()) => Response::Ok,
                    Err(e) => Response::Error { message: e },
                }
            }
            None => Response::Error { message: "no plugin loaded".into() },
        },
        Request::CrashForTesting => std::process::abort(),
        Request::Shutdown => {
            SHUTDOWN.with(|s| s.set(true));
            Response::Ok
        }
    }
}
