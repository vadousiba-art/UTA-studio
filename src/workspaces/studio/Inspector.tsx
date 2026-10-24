import { Copy, Layers, Scissors, Trash2 } from "lucide-react";
import type { StudioState } from "../../api/types";
import { Button, Field, Notice } from "../../components/ui";
import { formatTime } from "../../lib/music";
import { useStudioActions } from "./useStudio";

/** Clip properties and, for take groups, the comp editor. */
export function Inspector({
  state,
  clipId,
  playhead,
  selection,
}: {
  state: StudioState;
  clipId: string | null;
  playhead: number;
  selection: [number, number] | null;
}) {
  const { edit, change } = useStudioActions();
  const p = state.project;
  const sr = p.sampleRate;
  const track = p.tracks.find((t) => t.clips.some((c) => c.id === clipId));
  const clip = track?.clips.find((c) => c.id === clipId);
  if (!track || !clip) return <div className="muted">Select a clip on the timeline to edit it.</div>;
  const group = track.takeGroups.find((g) => g.takes.includes(clip.id));
  const inClip = playhead * sr > clip.position && playhead * sr < clip.position + clip.length;

  if (group) {
    const range: [number, number] | null = selection ? [Math.round(selection[0] * sr), Math.round(selection[1] * sr)] : null;
    return (
      <div className="stack">
        <div className="row row--between">
          <strong>
            <Layers size={14} style={{ verticalAlign: -2 }} /> {group.takes.length} takes on {track.name}
          </strong>
          <Button size="sm" onClick={() => void edit({ op: "flattenComp", track: track.id, group: group.id })} title="Replace the takes with plain clips that follow the current comp">
            Flatten comp
          </Button>
        </div>
        {!range && <Notice kind="info">Drag across the timeline to select a range, then choose which take plays there.</Notice>}
        <table className="table">
          <thead>
            <tr>
              <th>Take</th>
              <th>Recorded at</th>
              <th>Length</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {group.takes.map((id, i) => {
              const c = track.clips.find((x) => x.id === id);
              if (!c) return null;
              const playing = group.comp.filter((s) => s.take === id).reduce((n, s) => n + (s.end - s.start), 0);
              return (
                <tr key={id}>
                  <td>
                    Take {i + 1} {playing > 0 && <span className="tag tag--accent">in comp {formatTime(playing / sr)}</span>}
                  </td>
                  <td className="mono">{formatTime(c.position / sr, true)}</td>
                  <td className="mono">{formatTime(c.length / sr, true)}</td>
                  <td style={{ textAlign: "right" }}>
                    <Button size="sm" disabled={!range} onClick={() => range && void edit({ op: "setComp", track: track.id, group: group.id, start: range[0], end: range[1], take: id })}>
                      Use for selection
                    </Button>{" "}
                    <Button size="sm" onClick={() => void edit({ op: "setComp", track: track.id, group: group.id, start: c.position, end: c.position + c.length, take: id })}>
                      Use whole take
                    </Button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <div className="stack">
      <div className="row row--between">
        <strong>{clip.name}</strong>
        <div className="row">
          <Button size="sm" icon={<Scissors size={12} />} disabled={!inClip} title="Split at the playhead (S)" onClick={() => void edit({ op: "splitClip", clip: clip.id, at: Math.round(playhead * sr) })}>
            Split
          </Button>
          <Button size="sm" icon={<Copy size={12} />} title="Duplicate (Ctrl+D)" onClick={() => void edit({ op: "duplicateClip", clip: clip.id, position: null })}>
            Duplicate
          </Button>
          <Button size="sm" variant="danger" icon={<Trash2 size={12} />} title="Delete (Del)" onClick={() => void edit({ op: "deleteClip", clip: clip.id })}>
            Delete
          </Button>
        </div>
      </div>
      <div className="faint mono" style={{ fontSize: 11 }}>
        starts {formatTime(clip.position / sr, true)} · length {formatTime(clip.length / sr, true)} · source offset {formatTime(clip.sourceOffset / sr, true)}
      </div>
      <div className="grid grid--3">
        <Field label={`Clip gain ${clip.gainDb > 0 ? "+" : ""}${clip.gainDb.toFixed(1)} dB`}>
          <input
            className="slider"
            type="range"
            min={-24}
            max={12}
            step={0.5}
            defaultValue={clip.gainDb}
            key={`g${clip.id}${clip.gainDb}`}
            onPointerUp={(e) => change({ op: "updateClip", clip: clip.id, update: { gainDb: Number((e.target as HTMLInputElement).value) } }, true)}
            onKeyUp={(e) => change({ op: "updateClip", clip: clip.id, update: { gainDb: Number((e.target as HTMLInputElement).value) } }, true)}
          />
        </Field>
        <Field label={`Fade in ${(clip.fadeIn / sr * 1000).toFixed(0)} ms`}>
          <input
            className="slider"
            type="range"
            min={0}
            max={Math.min(2, clip.length / sr / 2)}
            step={0.005}
            defaultValue={clip.fadeIn / sr}
            key={`fi${clip.id}${clip.fadeIn}`}
            onPointerUp={(e) => change({ op: "updateClip", clip: clip.id, update: { fadeIn: Math.round(Number((e.target as HTMLInputElement).value) * sr) } }, true)}
          />
        </Field>
        <Field label={`Fade out ${(clip.fadeOut / sr * 1000).toFixed(0)} ms`}>
          <input
            className="slider"
            type="range"
            min={0}
            max={Math.min(2, clip.length / sr / 2)}
            step={0.005}
            defaultValue={clip.fadeOut / sr}
            key={`fo${clip.id}${clip.fadeOut}`}
            onPointerUp={(e) => change({ op: "updateClip", clip: clip.id, update: { fadeOut: Math.round(Number((e.target as HTMLInputElement).value) * sr) } }, true)}
          />
        </Field>
      </div>
      <div className="row">
        <Button size="sm" aria-pressed={clip.muted} onClick={() => void edit({ op: "updateClip", clip: clip.id, update: { muted: !clip.muted } })}>
          {clip.muted ? "Clip muted" : "Mute clip"}
        </Button>
      </div>
    </div>
  );
}
