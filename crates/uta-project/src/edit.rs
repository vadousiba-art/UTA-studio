//! Editing operations on a [`Project`]. Each operation validates its inputs and
//! either applies completely or returns an error leaving the project
//! unchanged. Undo/redo is provided by [`crate::history::History`], which
//! snapshots the document around these calls.

use std::collections::BTreeMap;

use crate::error::EditError;
use crate::model::*;

type Result<T> = std::result::Result<T, EditError>;

fn track_mut(p: &mut Project, id: Id) -> Result<&mut Track> {
    p.track_mut(id).ok_or(EditError::TrackNotFound(id))
}

fn clip_mut(p: &mut Project, id: Id) -> Result<&mut Clip> {
    let (ti, ci) = p.find_clip(id).ok_or(EditError::ClipNotFound(id))?;
    Ok(&mut p.tracks[ti].clips[ci])
}

pub fn add_track(p: &mut Project, name: Option<String>) -> Id {
    let n = p.tracks.len();
    let name = name.unwrap_or_else(|| format!("Track {}", n + 1));
    let color = TRACK_COLORS[n % TRACK_COLORS.len()];
    let t = Track::new(name, color);
    let id = t.id;
    p.tracks.push(t);
    id
}

pub fn remove_track(p: &mut Project, id: Id) -> Result<Track> {
    let idx = p.tracks.iter().position(|t| t.id == id).ok_or(EditError::TrackNotFound(id))?;
    Ok(p.tracks.remove(idx))
}

pub fn move_track(p: &mut Project, id: Id, new_index: usize) -> Result<()> {
    let idx = p.tracks.iter().position(|t| t.id == id).ok_or(EditError::TrackNotFound(id))?;
    let t = p.tracks.remove(idx);
    let ni = new_index.min(p.tracks.len());
    p.tracks.insert(ni, t);
    Ok(())
}

/// Partial update of track properties.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackUpdate {
    pub name: Option<String>,
    pub color: Option<String>,
    pub gain_db: Option<f32>,
    pub pan: Option<f32>,
    pub mute: Option<bool>,
    pub solo: Option<bool>,
    pub armed: Option<bool>,
    pub monitor: Option<MonitorMode>,
    pub input_channel: Option<u16>,
    pub input_stereo: Option<bool>,
    pub height: Option<u16>,
}

pub fn update_track(p: &mut Project, id: Id, u: &TrackUpdate) -> Result<()> {
    if let Some(g) = u.gain_db {
        if !g.is_finite() {
            return Err(EditError::Invalid("gain must be finite".into()));
        }
    }
    let t = track_mut(p, id)?;
    if let Some(v) = &u.name {
        let v = v.trim();
        if v.is_empty() {
            return Err(EditError::Invalid("track name cannot be empty".into()));
        }
        t.name = v.chars().take(64).collect();
    }
    if let Some(v) = &u.color {
        t.color = v.clone();
    }
    if let Some(v) = u.gain_db {
        t.gain_db = v.clamp(-96.0, 12.0);
    }
    if let Some(v) = u.pan {
        t.pan = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
    }
    if let Some(v) = u.mute {
        t.mute = v;
    }
    if let Some(v) = u.solo {
        t.solo = v;
    }
    if let Some(v) = u.armed {
        t.armed = v;
    }
    if let Some(v) = u.monitor {
        t.monitor = v;
    }
    if let Some(v) = u.input_channel {
        t.input_channel = v;
    }
    if let Some(v) = u.input_stereo {
        t.input_stereo = v;
    }
    if let Some(v) = u.height {
        t.height = v.clamp(48, 400);
    }
    Ok(())
}

/// Place a new clip for an asset on a track. Returns the clip id.
pub fn add_clip(p: &mut Project, track: Id, asset: Id, position: u64) -> Result<Id> {
    let a = p.asset(asset).ok_or(EditError::AssetNotFound(asset))?.clone();
    if a.frames == 0 {
        return Err(EditError::Invalid("asset is empty".into()));
    }
    let t = track_mut(p, track)?;
    let clip = Clip {
        id: new_id(),
        asset_id: asset,
        name: a.original_name.clone(),
        position,
        source_offset: 0,
        length: a.frames,
        gain_db: 0.0,
        fade_in: 0,
        fade_out: 0,
        muted: false,
    };
    let id = clip.id;
    t.clips.push(clip);
    t.clips.sort_by_key(|c| c.position);
    Ok(id)
}

/// Move a clip to a new position, optionally onto another track.
pub fn move_clip(p: &mut Project, clip: Id, position: u64, to_track: Option<Id>) -> Result<()> {
    let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
    if p.tracks[ti].take_group_of(clip).is_some() {
        return Err(EditError::Invalid("takes in a take group move together; flatten the comp first".into()));
    }
    match to_track {
        Some(dest) if dest != p.tracks[ti].id => {
            let di = p.tracks.iter().position(|t| t.id == dest).ok_or(EditError::TrackNotFound(dest))?;
            let mut c = p.tracks[ti].clips.remove(ci);
            c.position = position;
            p.tracks[di].clips.push(c);
            p.tracks[di].clips.sort_by_key(|c| c.position);
        }
        _ => {
            p.tracks[ti].clips[ci].position = position;
            p.tracks[ti].clips.sort_by_key(|c| c.position);
        }
    }
    Ok(())
}

/// Trim a clip's start and/or end on the timeline. `new_start`/`new_end` are
/// timeline frames. The clip can never extend beyond its asset.
pub fn trim_clip(p: &mut Project, clip: Id, new_start: Option<u64>, new_end: Option<u64>) -> Result<()> {
    let asset_frames = {
        let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
        let c = &p.tracks[ti].clips[ci];
        p.asset(c.asset_id).map(|a| a.frames).ok_or(EditError::AssetNotFound(c.asset_id))?
    };
    let c = clip_mut(p, clip)?;
    let mut start = c.position;
    let mut offset = c.source_offset;
    let mut end = c.end();
    if let Some(s) = new_start {
        // earliest start: where the asset begins
        let earliest = c.position.saturating_sub(c.source_offset);
        let s = s.max(earliest);
        if s >= end {
            return Err(EditError::Invalid("trim would make the clip empty".into()));
        }
        // s >= earliest guarantees the new offset is non-negative
        offset = (c.source_offset as i64 + s as i64 - c.position as i64) as u64;
        start = s;
    }
    if let Some(e) = new_end {
        let latest = start + (asset_frames - offset);
        let e = e.min(latest);
        if e <= start {
            return Err(EditError::Invalid("trim would make the clip empty".into()));
        }
        end = e;
    }
    c.position = start;
    c.source_offset = offset;
    c.length = end - start;
    c.fade_in = c.fade_in.min(c.length / 2);
    c.fade_out = c.fade_out.min(c.length / 2);
    Ok(())
}

/// Split a clip at a timeline position. Returns the id of the right-hand part.
pub fn split_clip(p: &mut Project, clip: Id, at: u64) -> Result<Id> {
    let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
    if p.tracks[ti].take_group_of(clip).is_some() {
        return Err(EditError::Invalid("split takes through the comp editor".into()));
    }
    let c = p.tracks[ti].clips[ci].clone();
    if at <= c.position || at >= c.end() {
        return Err(EditError::Invalid("split point must be inside the clip".into()));
    }
    let left_len = at - c.position;
    let mut right = c.clone();
    right.id = new_id();
    right.position = at;
    right.source_offset = c.source_offset + left_len;
    right.length = c.length - left_len;
    right.fade_in = 0;
    right.fade_out = c.fade_out.min(right.length / 2);
    let left = &mut p.tracks[ti].clips[ci];
    left.length = left_len;
    left.fade_out = 0;
    left.fade_in = left.fade_in.min(left_len / 2);
    let rid = right.id;
    p.tracks[ti].clips.insert(ci + 1, right);
    Ok(rid)
}

/// Duplicate a clip, placing the copy right after the original (or at
/// `position`). Returns the new clip id.
pub fn duplicate_clip(p: &mut Project, clip: Id, position: Option<u64>) -> Result<Id> {
    let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
    let mut c = p.tracks[ti].clips[ci].clone();
    c.id = new_id();
    c.position = position.unwrap_or(c.end());
    let id = c.id;
    p.tracks[ti].clips.push(c);
    p.tracks[ti].clips.sort_by_key(|c| c.position);
    Ok(id)
}

/// Delete a clip from the timeline. The asset file is kept (it may be used by
/// other clips, and deletion of recordings is a separate, explicit action).
pub fn delete_clip(p: &mut Project, clip: Id) -> Result<Clip> {
    let (ti, ci) = p.find_clip(clip).ok_or(EditError::ClipNotFound(clip))?;
    let t = &mut p.tracks[ti];
    let removed = t.clips.remove(ci);
    // remove from take groups; drop groups with fewer than two takes
    for g in &mut t.take_groups {
        g.takes.retain(|id| *id != clip);
    }
    let fallback: BTreeMap<Id, Option<Id>> = t
        .take_groups
        .iter()
        .map(|g| (g.id, g.takes.last().copied()))
        .collect();
    for g in &mut t.take_groups {
        if let Some(Some(fb)) = fallback.get(&g.id) {
            for s in &mut g.comp {
                if s.take == clip {
                    s.take = *fb;
                }
            }
        }
    }
    t.take_groups.retain(|g| g.takes.len() >= 2);
    Ok(removed)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipUpdate {
    pub name: Option<String>,
    pub gain_db: Option<f32>,
    pub fade_in: Option<u64>,
    pub fade_out: Option<u64>,
    pub muted: Option<bool>,
}

pub fn update_clip(p: &mut Project, clip: Id, u: &ClipUpdate) -> Result<()> {
    let c = clip_mut(p, clip)?;
    if let Some(n) = &u.name {
        c.name = n.chars().take(64).collect();
    }
    if let Some(g) = u.gain_db {
        if !g.is_finite() {
            return Err(EditError::Invalid("gain must be finite".into()));
        }
        c.gain_db = g.clamp(-48.0, 24.0);
    }
    if let Some(f) = u.fade_in {
        c.fade_in = f.min(c.length / 2);
    }
    if let Some(f) = u.fade_out {
        c.fade_out = f.min(c.length / 2);
    }
    if let Some(m) = u.muted {
        c.muted = m;
    }
    Ok(())
}

// ---------------------------------------------------------------- takes ----

/// Register a newly recorded clip as a take. If it overlaps an existing clip
/// or take group on the same track, the clips form (or join) a take group and
/// the new take becomes the active comp over its range.
pub fn register_take(p: &mut Project, track: Id, new_clip: Id) -> Result<Option<Id>> {
    let t = track_mut(p, track)?;
    let nc = t.clip(new_clip).ok_or(EditError::ClipNotFound(new_clip))?.clone();
    let overlaps = |c: &Clip| c.id != nc.id && c.position < nc.end() && nc.position < c.end();
    // existing group that overlaps?
    let group_idx = t.take_groups.iter().position(|g| {
        g.takes
            .iter()
            .filter_map(|id| t.clips.iter().find(|c| c.id == *id))
            .any(overlaps)
    });
    let gid = match group_idx {
        Some(gi) => {
            let g = &mut t.take_groups[gi];
            g.takes.push(nc.id);
            g.id
        }
        None => {
            let others: Vec<Id> = t
                .clips
                .iter()
                .filter(|c| overlaps(c) && !t.take_groups.iter().any(|g| g.takes.contains(&c.id)))
                .map(|c| c.id)
                .collect();
            if others.is_empty() {
                return Ok(None);
            }
            let mut comp = Vec::new();
            for id in &others {
                if let Some(c) = t.clips.iter().find(|c| c.id == *id) {
                    comp.push(CompSegment { start: c.position, end: c.end(), take: c.id });
                }
            }
            let mut takes = others;
            takes.push(nc.id);
            let g = TakeGroup { id: new_id(), takes, comp };
            let id = g.id;
            t.take_groups.push(g);
            id
        }
    };
    // new take wins over its own range
    set_comp(p, track, gid, nc.position, nc.end(), nc.id)?;
    Ok(Some(gid))
}

/// Choose which take plays in `[start, end)` of a take group.
pub fn set_comp(p: &mut Project, track: Id, group: Id, start: u64, end: u64, take: Id) -> Result<()> {
    if end <= start {
        return Err(EditError::Invalid("empty comp range".into()));
    }
    let t = track_mut(p, track)?;
    let take_clip = t.clip(take).ok_or(EditError::ClipNotFound(take))?.clone();
    let g = t
        .take_groups
        .iter_mut()
        .find(|g| g.id == group)
        .ok_or(EditError::Invalid("take group not found".into()))?;
    if !g.takes.contains(&take) {
        return Err(EditError::Invalid("clip is not a take of this group".into()));
    }
    // the chosen take can only cover where it has audio
    let start = start.max(take_clip.position);
    let end = end.min(take_clip.end());
    if end <= start {
        return Err(EditError::Invalid("take has no audio in that range".into()));
    }
    let mut out: Vec<CompSegment> = Vec::new();
    for s in g.comp.drain(..) {
        if s.end <= start || s.start >= end {
            out.push(s);
            continue;
        }
        if s.start < start {
            out.push(CompSegment { start: s.start, end: start, take: s.take });
        }
        if s.end > end {
            out.push(CompSegment { start: end, end: s.end, take: s.take });
        }
    }
    out.push(CompSegment { start, end, take });
    out.sort_by_key(|s| s.start);
    // merge adjacent segments using the same take
    let mut merged: Vec<CompSegment> = Vec::new();
    for s in out {
        match merged.last_mut() {
            Some(last) if last.take == s.take && last.end == s.start => last.end = s.end,
            _ => merged.push(s),
        }
    }
    g.comp = merged;
    Ok(())
}

/// Replace a take group with plain clips matching its comp (one clip per
/// comp segment). The original takes are removed from the timeline but their
/// assets remain in the project.
pub fn flatten_comp(p: &mut Project, track: Id, group: Id) -> Result<Vec<Id>> {
    let t = track_mut(p, track)?;
    let gi = t
        .take_groups
        .iter()
        .position(|g| g.id == group)
        .ok_or(EditError::Invalid("take group not found".into()))?;
    let g = t.take_groups.remove(gi);
    let mut new_ids = Vec::new();
    let mut new_clips = Vec::new();
    for s in &g.comp {
        if let Some(c) = t.clips.iter().find(|c| c.id == s.take) {
            let mut nc = c.clone();
            nc.id = new_id();
            nc.source_offset = c.source_offset + (s.start - c.position);
            nc.position = s.start;
            nc.length = s.end - s.start;
            nc.fade_in = 0;
            nc.fade_out = 0;
            new_ids.push(nc.id);
            new_clips.push(nc);
        }
    }
    t.clips.retain(|c| !g.takes.contains(&c.id));
    t.clips.extend(new_clips);
    t.clips.sort_by_key(|c| c.position);
    Ok(new_ids)
}

// -------------------------------------------------------------- effects ----

/// `None` = master bus.
fn chain_mut(p: &mut Project, track: Option<Id>) -> Result<&mut Vec<EffectSlot>> {
    match track {
        Some(id) => Ok(&mut track_mut(p, id)?.effects),
        None => Ok(&mut p.master.effects),
    }
}

pub const MAX_EFFECTS_PER_CHAIN: usize = 16;

pub fn insert_effect(p: &mut Project, track: Option<Id>, slot: EffectSlot, index: Option<usize>) -> Result<Id> {
    let chain = chain_mut(p, track)?;
    if chain.len() >= MAX_EFFECTS_PER_CHAIN {
        return Err(EditError::Invalid(format!("at most {MAX_EFFECTS_PER_CHAIN} effects per chain")));
    }
    let id = slot.id;
    let i = index.unwrap_or(chain.len()).min(chain.len());
    chain.insert(i, slot);
    Ok(id)
}

pub fn remove_effect(p: &mut Project, track: Option<Id>, slot: Id) -> Result<EffectSlot> {
    let chain = chain_mut(p, track)?;
    let i = chain.iter().position(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    Ok(chain.remove(i))
}

pub fn move_effect(p: &mut Project, track: Option<Id>, slot: Id, new_index: usize) -> Result<()> {
    let chain = chain_mut(p, track)?;
    let i = chain.iter().position(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    let s = chain.remove(i);
    let ni = new_index.min(chain.len());
    chain.insert(ni, s);
    Ok(())
}

pub fn set_effect_bypass(p: &mut Project, track: Option<Id>, slot: Id, bypass: bool) -> Result<()> {
    let chain = chain_mut(p, track)?;
    let s = chain.iter_mut().find(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    s.bypass = bypass;
    Ok(())
}

/// Set a built-in effect parameter (value is clamped to the declared range).
pub fn set_builtin_param(p: &mut Project, track: Option<Id>, slot: Id, param: &str, value: f32) -> Result<f32> {
    let chain = chain_mut(p, track)?;
    let s = chain.iter_mut().find(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    match &mut s.effect {
        EffectKind::Builtin { kind, params } => {
            let info = kind
                .params()
                .iter()
                .find(|i| i.id == param)
                .ok_or_else(|| EditError::Invalid(format!("unknown parameter {param}")))?;
            let v = info.clamp(value);
            params.insert(param.to_string(), v);
            Ok(v)
        }
        EffectKind::Plugin { .. } => Err(EditError::Invalid("not a built-in effect".into())),
    }
}

pub fn set_plugin_state(
    p: &mut Project,
    track: Option<Id>,
    slot: Id,
    new_state: Option<String>,
    new_params: Option<BTreeMap<u32, f64>>,
) -> Result<()> {
    let chain = chain_mut(p, track)?;
    let s = chain.iter_mut().find(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    match &mut s.effect {
        EffectKind::Plugin { state, params, .. } => {
            if let Some(st) = new_state {
                *state = Some(st);
            }
            if let Some(pp) = new_params {
                *params = pp;
            }
            Ok(())
        }
        EffectKind::Builtin { .. } => Err(EditError::Invalid("not a plugin".into())),
    }
}

/// Replace the plugin reference of a slot (relinking a missing plugin),
/// keeping its stored state.
pub fn relink_plugin(p: &mut Project, track: Option<Id>, slot: Id, new_ref: PluginRef) -> Result<()> {
    let chain = chain_mut(p, track)?;
    let s = chain.iter_mut().find(|s| s.id == slot).ok_or(EditError::EffectNotFound(slot))?;
    match &mut s.effect {
        EffectKind::Plugin { plugin, .. } => {
            *plugin = new_ref;
            Ok(())
        }
        EffectKind::Builtin { .. } => Err(EditError::Invalid("not a plugin".into())),
    }
}

pub fn set_master_gain(p: &mut Project, gain_db: f32) -> Result<()> {
    if !gain_db.is_finite() {
        return Err(EditError::Invalid("gain must be finite".into()));
    }
    p.master.gain_db = gain_db.clamp(-96.0, 12.0);
    Ok(())
}

pub fn set_loop(p: &mut Project, region: Option<Region>) -> Result<()> {
    if let Some(r) = region {
        if r.is_empty() {
            return Err(EditError::Invalid("loop region is empty".into()));
        }
    }
    p.loop_region = region;
    Ok(())
}

pub fn set_tempo(p: &mut Project, bpm: f32) -> Result<()> {
    if !(20.0..=300.0).contains(&bpm) {
        return Err(EditError::Invalid("tempo must be 20–300 BPM".into()));
    }
    p.tempo_bpm = bpm;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn project_with_clip() -> (Project, Id, Id, Id) {
        let mut p = Project::new("t", 48_000);
        let tr = add_track(&mut p, None);
        let asset = Asset {
            id: new_id(),
            file: "audio/a.wav".into(),
            original_name: "a".into(),
            kind: AssetKind::Recording,
            sample_rate: 48_000,
            channels: 1,
            frames: 48_000,
            created_at: Utc::now(),
        };
        let aid = asset.id;
        p.assets.push(asset);
        let c = add_clip(&mut p, tr, aid, 1000).unwrap();
        (p, tr, aid, c)
    }

    #[test]
    fn split_preserves_audio_mapping() {
        let (mut p, tr, _, c) = project_with_clip();
        let r = split_clip(&mut p, c, 11_000).unwrap();
        let t = p.track(tr).unwrap();
        let l = t.clip(c).unwrap();
        let rc = t.clip(r).unwrap();
        assert_eq!((l.position, l.length, l.source_offset), (1000, 10_000, 0));
        assert_eq!((rc.position, rc.length, rc.source_offset), (11_000, 38_000, 10_000));
        assert!(split_clip(&mut p, c, 1000).is_err());
    }

    #[test]
    fn trim_clamps_to_asset() {
        let (mut p, _, _, c) = project_with_clip();
        trim_clip(&mut p, c, Some(5000), Some(20_000)).unwrap();
        let (ti, ci) = p.find_clip(c).unwrap();
        let cl = &p.tracks[ti].clips[ci];
        assert_eq!((cl.position, cl.source_offset, cl.length), (5000, 4000, 15_000));
        // extend start before asset start -> clamps to asset start
        trim_clip(&mut p, c, Some(0), None).unwrap();
        let cl = &p.tracks[ti].clips[ci];
        assert_eq!((cl.position, cl.source_offset), (1000, 0));
        // extend end beyond asset -> clamps
        trim_clip(&mut p, c, None, Some(1_000_000)).unwrap();
        let cl = &p.tracks[ti].clips[ci];
        assert_eq!(cl.end(), 49_000);
        assert!(trim_clip(&mut p, c, Some(49_000), None).is_err());
    }

    #[test]
    fn move_between_tracks_and_duplicate() {
        let (mut p, _, _, c) = project_with_clip();
        let t2 = add_track(&mut p, Some("Vox 2".into()));
        move_clip(&mut p, c, 2000, Some(t2)).unwrap();
        assert_eq!(p.track(t2).unwrap().clips.len(), 1);
        let d = duplicate_clip(&mut p, c, None).unwrap();
        assert_eq!(p.track(t2).unwrap().clip(d).unwrap().position, 50_000);
        delete_clip(&mut p, c).unwrap();
        assert!(p.find_clip(c).is_none());
        assert!(p.find_clip(d).is_some());
    }

    #[test]
    fn takes_and_comping() {
        let (mut p, tr, aid, c1) = project_with_clip();
        let c2 = add_clip(&mut p, tr, aid, 1000).unwrap();
        let g = register_take(&mut p, tr, c2).unwrap().expect("group created");
        {
            let grp = &p.track(tr).unwrap().take_groups[0];
            assert_eq!(grp.takes, vec![c1, c2]);
            assert_eq!(grp.comp, vec![CompSegment { start: 1000, end: 49_000, take: c2 }]);
        }
        // choose take 1 for the middle
        set_comp(&mut p, tr, g, 10_000, 20_000, c1).unwrap();
        let comp = p.track(tr).unwrap().take_groups[0].comp.clone();
        assert_eq!(comp.len(), 3);
        assert_eq!(comp[1], CompSegment { start: 10_000, end: 20_000, take: c1 });
        // a third take joins the existing group
        let c3 = add_clip(&mut p, tr, aid, 5000).unwrap();
        assert_eq!(register_take(&mut p, tr, c3).unwrap(), Some(g));
        assert_eq!(p.track(tr).unwrap().take_groups[0].takes.len(), 3);
        // flatten
        let ids = flatten_comp(&mut p, tr, g).unwrap();
        let t = p.track(tr).unwrap();
        assert!(t.take_groups.is_empty());
        assert_eq!(t.clips.len(), ids.len());
        // clips tile the comp without gaps
        for w in t.clips.windows(2) {
            assert_eq!(w[0].end(), w[1].position);
        }
    }

    #[test]
    fn effect_chain_edits() {
        let (mut p, tr, _, _) = project_with_clip();
        let eq = insert_effect(&mut p, Some(tr), EffectSlot::builtin(uta_dsp::BuiltinKind::Eq), None).unwrap();
        let comp = insert_effect(&mut p, Some(tr), EffectSlot::builtin(uta_dsp::BuiltinKind::Compressor), Some(0)).unwrap();
        assert_eq!(p.track(tr).unwrap().effects[0].id, comp);
        move_effect(&mut p, Some(tr), comp, 5).unwrap();
        assert_eq!(p.track(tr).unwrap().effects[1].id, comp);
        let v = set_builtin_param(&mut p, Some(tr), eq, "mid1_gain", 99.0).unwrap();
        assert_eq!(v, 18.0);
        assert!(set_builtin_param(&mut p, Some(tr), eq, "bogus", 1.0).is_err());
        set_effect_bypass(&mut p, Some(tr), eq, true).unwrap();
        remove_effect(&mut p, Some(tr), eq).unwrap();
        assert_eq!(p.track(tr).unwrap().effects.len(), 1);
        insert_effect(&mut p, None, EffectSlot::builtin(uta_dsp::BuiltinKind::Reverb), None).unwrap();
        assert_eq!(p.master.effects.len(), 1);
    }

    #[test]
    fn solo_logic() {
        let mut p = Project::new("s", 48_000);
        let a = add_track(&mut p, None);
        let b = add_track(&mut p, None);
        assert!(p.is_track_audible(p.track(a).unwrap()));
        update_track(&mut p, b, &TrackUpdate { solo: Some(true), ..Default::default() }).unwrap();
        assert!(!p.is_track_audible(p.track(a).unwrap()));
        assert!(p.is_track_audible(p.track(b).unwrap()));
        update_track(&mut p, b, &TrackUpdate { mute: Some(true), ..Default::default() }).unwrap();
        assert!(!p.is_track_audible(p.track(b).unwrap()));
    }
}
