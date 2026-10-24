//! Build a real-time [`Graph`] from a project document.

use uta_dsp::{create_builtin, db_to_gain};
use uta_project::{EffectKind, EffectSlot, MonitorMode, Project, Track};

use crate::core::Command;
use crate::graph::{pan_gains, BuiltinProcessor, ChainProcessor, ChainSlot, Graph, GraphTrack, InputRoute, PassThrough, MAX_BLOCK};

/// Creates processors for plugin slots. Returning `None` means the plugin is
/// unavailable; a pass-through is inserted so the chain keeps its shape.
pub trait PluginFactory {
    fn create(&mut self, track: Option<uuid::Uuid>, slot: &EffectSlot, sample_rate: f32) -> Option<Box<dyn ChainProcessor>>;
}

/// Factory used when plugin hosting is unavailable.
pub struct NoPlugins;

impl PluginFactory for NoPlugins {
    fn create(&mut self, _track: Option<uuid::Uuid>, _slot: &EffectSlot, _sample_rate: f32) -> Option<Box<dyn ChainProcessor>> {
        None
    }
}

pub fn build_processor(
    track: Option<uuid::Uuid>,
    slot: &EffectSlot,
    sample_rate: f32,
    plugins: &mut dyn PluginFactory,
) -> Box<dyn ChainProcessor> {
    match &slot.effect {
        EffectKind::Builtin { kind, params } => {
            let mut fx = create_builtin(*kind);
            for (id, v) in params {
                // unknown ids from newer versions are ignored
                fx.set_param_by_id(id, *v);
            }
            fx.prepare(sample_rate, MAX_BLOCK);
            Box::new(BuiltinProcessor(fx))
        }
        EffectKind::Plugin { .. } => plugins.create(track, slot, sample_rate).unwrap_or_else(|| Box::new(PassThrough)),
    }
}

fn build_chain(track: Option<uuid::Uuid>, slots: &[EffectSlot], sr: f32, plugins: &mut dyn PluginFactory) -> crate::graph::Chain {
    crate::graph::Chain {
        slots: slots
            .iter()
            .map(|s| ChainSlot {
                id: s.id.as_u128(),
                processor: build_processor(track, s, sr, plugins),
                bypass: s.bypass,
            })
            .collect(),
    }
}

/// Whether live input should pass through a track.
pub fn track_monitors(t: &Track) -> bool {
    match t.monitor {
        MonitorMode::Off => false,
        MonitorMode::On => true,
        MonitorMode::Armed => t.armed,
    }
}

pub fn build_graph(project: &Project, sample_rate: f32, plugins: &mut dyn PluginFactory) -> Graph {
    let mut g = Graph::new(sample_rate);
    for (i, t) in project.tracks.iter().enumerate().take(crate::shared::MAX_TRACKS) {
        let mut gt = GraphTrack::new(t.id.as_u128(), i, sample_rate);
        gt.set_gain(db_to_gain(t.gain_db), true);
        gt.pan = pan_gains(t.pan);
        gt.audible = project.is_track_audible(t);
        gt.monitor = track_monitors(t);
        gt.input = InputRoute { channel: t.input_channel, stereo: t.input_stereo };
        gt.chain = build_chain(Some(t.id), &t.effects, sample_rate, plugins);
        g.tracks.push(gt);
    }
    g.master = build_chain(None, &project.master.effects, sample_rate, plugins);
    g.compensate_latency();
    g.master_gain.snap(db_to_gain(project.master.gain_db));
    g
}

/// Commands that bring an existing graph's mix settings in line with the
/// project (gain, pan, mute/solo, monitoring) without rebuilding it.
pub fn mix_commands(project: &Project) -> Vec<Command> {
    let mut v: Vec<Command> = project
        .tracks
        .iter()
        .map(|t| Command::SetTrackMix {
            id: t.id.as_u128(),
            gain: db_to_gain(t.gain_db),
            pan: pan_gains(t.pan),
            audible: project.is_track_audible(t),
            monitor: track_monitors(t),
            input: InputRoute { channel: t.input_channel, stereo: t.input_stereo },
        })
        .collect();
    v.push(Command::SetMasterGain(db_to_gain(project.master.gain_db)));
    v
}

/// A structural fingerprint: when it changes, the graph must be rebuilt
/// (tracks added/removed/reordered, effects inserted/removed/moved).
pub fn structure_key(project: &Project) -> Vec<u128> {
    let mut k = Vec::new();
    for t in &project.tracks {
        k.push(t.id.as_u128());
        for e in &t.effects {
            k.push(e.id.as_u128());
        }
        k.push(0);
    }
    for e in &project.master.effects {
        k.push(e.id.as_u128());
    }
    k
}
