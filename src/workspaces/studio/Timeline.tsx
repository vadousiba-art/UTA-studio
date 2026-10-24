import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api/client";
import type { Clip, StudioState, Track } from "../../api/types";
import { formatTime } from "../../lib/music";
import { useApp } from "../../state/app";
import { Meter } from "../../viz/Meter";
import { useStudioActions } from "./useStudio";

export const HEADER_W = 230;
const RULER_H = 28;

export interface TimelineProps {
  state: StudioState;
  pps: number;
  snap: "off" | "beat" | "bar";
  selectedClip: string | null;
  onSelectClip: (id: string | null, track: string | null) => void;
  selectedTrack: string | null;
  onSelectTrack: (id: string) => void;
  selection: [number, number] | null;
  onSelection: (s: [number, number] | null) => void;
  playhead: number;
  recordingSince: number | null;
}

const waveCache = new Map<string, number[]>();

function ClipWave({ assetId, start, end, width, height, color }: { assetId: string; start: number; end: number; width: number; height: number; color: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const buckets = Math.max(8, Math.min(3000, Math.floor(width)));
  const key = `${assetId}:${start}:${end}:${buckets}`;
  const [peaks, setPeaks] = useState<number[] | null>(waveCache.get(key) ?? null);
  useEffect(() => {
    if (waveCache.has(key)) {
      setPeaks(waveCache.get(key)!);
      return;
    }
    let alive = true;
    const t = window.setTimeout(() => {
      api
        .studioAssetWaveform(assetId, start, end, buckets)
        .then((p) => {
          if (waveCache.size > 400) waveCache.clear();
          waveCache.set(key, p);
          if (alive) setPeaks(p);
        })
        .catch(() => alive && setPeaks([]));
    }, 60);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [key, assetId, start, end, buckets]);
  useEffect(() => {
    const c = canvas.current;
    if (!c || !peaks) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.max(1, Math.floor(width * dpr));
    c.height = Math.floor(height * dpr);
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = color;
    const n = peaks.length / 2;
    const mid = height / 2;
    for (let i = 0; i < n; i++) {
      const x = (i / n) * width;
      const mn = peaks[2 * i]!;
      const mx = peaks[2 * i + 1]!;
      ctx.fillRect(x, mid - mx * mid * 0.95, Math.max(1, width / n), Math.max(1, (mx - mn) * mid * 0.95));
    }
  }, [peaks, width, height, color]);
  return <canvas ref={canvas} style={{ width, height, display: "block", pointerEvents: "none" }} />;
}

type Drag =
  | { kind: "move"; clip: Clip; track: string; x0: number; y0: number; pos: number; targetTrack: string }
  | { kind: "trimStart" | "trimEnd"; clip: Clip; x0: number; start: number; end: number }
  | { kind: "select"; t0: number };

export function Timeline(props: TimelineProps) {
  const { state, pps, snap, selectedClip, onSelectClip, selectedTrack, onSelectTrack, selection, onSelection, playhead, recordingSince } = props;
  const p = state.project;
  const sr = p.sampleRate;
  const status = useApp((s) => s.status);
  const error = useApp((s) => s.error);
  const { edit, change } = useStudioActions();
  const scroller = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const beat = 60 / p.tempoBpm;
  const bar = beat * p.timeSignature[0];
  const grid = snap === "beat" ? beat : snap === "bar" ? bar : 0;
  const snapT = (t: number) => (grid > 0 ? Math.round(t / grid) * grid : t);
  const total = Math.max(p.tracks.reduce((m, t) => Math.max(m, ...t.clips.map((c) => (c.position + c.length) / sr)), 0) + 30, 120, playhead + 20);
  const width = total * pps;

  // keep the playhead visible while playing
  useEffect(() => {
    const el = scroller.current;
    if (!el || !status?.playing) return;
    const x = HEADER_W + playhead * pps;
    if (x > el.scrollLeft + el.clientWidth - 60 || x < el.scrollLeft + HEADER_W) el.scrollLeft = Math.max(0, x - HEADER_W - 80);
  }, [playhead, pps, status?.playing]);

  const timeAt = (clientX: number) => {
    const el = scroller.current!;
    const r = el.getBoundingClientRect();
    return Math.max(0, (clientX - r.left + el.scrollLeft - HEADER_W) / pps);
  };

  const groupOf = (t: Track, clipId: string) => t.takeGroups.find((g) => g.takes.includes(clipId));

  const rows = useMemo(() => {
    let y = RULER_H;
    return p.tracks.map((t) => {
      const r = { track: t, y, h: t.height };
      y += t.height;
      return r;
    });
  }, [p.tracks]);

  const trackAtY = (clientY: number): string | null => {
    const el = scroller.current!;
    const r = el.getBoundingClientRect();
    const y = clientY - r.top + el.scrollTop;
    return rows.find((row) => y >= row.y && y < row.y + row.h)?.track.id ?? null;
  };

  const onMove = (e: React.PointerEvent) => {
    if (!drag) return;
    if (drag.kind === "select") {
      const t = snapT(timeAt(e.clientX));
      onSelection(Math.abs(t - drag.t0) > 0.02 ? [Math.min(t, drag.t0), Math.max(t, drag.t0)] : null);
    } else if (drag.kind === "move") {
      const dt = (e.clientX - drag.x0) / pps;
      const pos = snapT(Math.max(0, drag.clip.position / sr + dt));
      const tt = trackAtY(e.clientY) ?? drag.targetTrack;
      setDrag({ ...drag, pos, targetTrack: groupOf(p.tracks.find((t) => t.id === drag.track)!, drag.clip.id) ? drag.track : tt });
    } else {
      const dt = (e.clientX - drag.x0) / pps;
      const c = drag.clip;
      if (drag.kind === "trimStart") {
        const earliest = (c.position - c.sourceOffset) / sr;
        setDrag({ ...drag, start: Math.min(drag.end - 0.02, Math.max(earliest, snapT(c.position / sr + dt))) });
      } else {
        setDrag({ ...drag, end: Math.max(drag.start + 0.02, snapT((c.position + c.length) / sr + dt)) });
      }
    }
  };

  const onUp = () => {
    if (!drag) return;
    if (drag.kind === "move") {
      const newPos = Math.round(drag.pos * sr);
      if (newPos !== drag.clip.position || drag.targetTrack !== drag.track) {
        void edit({ op: "moveClip", clip: drag.clip.id, position: newPos, track: drag.targetTrack !== drag.track ? drag.targetTrack : null });
      }
    } else if (drag.kind === "trimStart" || drag.kind === "trimEnd") {
      const s = Math.round(drag.start * sr);
      const en = Math.round(drag.end * sr);
      if (s !== drag.clip.position || en !== drag.clip.position + drag.clip.length) {
        void edit({ op: "trimClip", clip: drag.clip.id, start: drag.kind === "trimStart" ? s : null, end: drag.kind === "trimEnd" ? en : null });
      }
    }
    setDrag(null);
  };

  const ticks = useMemo(() => {
    const out: { x: number; label?: string; major: boolean }[] = [];
    const showBeats = beat * pps > 18;
    const nBars = Math.ceil(total / bar);
    for (let b = 0; b <= nBars; b++) {
      out.push({ x: b * bar * pps, label: bar * pps > 34 || b % 4 === 0 ? String(b + 1) : undefined, major: true });
      if (showBeats) for (let k = 1; k < p.timeSignature[0]; k++) out.push({ x: (b * bar + k * beat) * pps, major: false });
    }
    return out;
  }, [beat, bar, pps, total, p.timeSignature]);

  const loop = p.loopRegion;
  return (
    <div
      ref={scroller}
      style={{ flex: 1, overflow: "auto", position: "relative", background: "var(--bg)" }}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerLeave={onUp}
    >
      <div style={{ width: width + HEADER_W, position: "relative", minHeight: "100%" }}>
        {/* ruler */}
        <div style={{ position: "sticky", top: 0, zIndex: 4, height: RULER_H, display: "flex" }}>
          <div style={{ position: "sticky", left: 0, width: HEADER_W, flex: "none", background: "var(--panel)", borderBottom: "1px solid var(--border)", borderRight: "1px solid var(--border)", zIndex: 5, display: "flex", alignItems: "center", padding: "0 10px" }} className="mono faint">
            {formatTime(playhead, true)} · bar {Math.floor(playhead / bar) + 1}.{Math.floor((playhead % bar) / beat) + 1}
          </div>
          <div
            style={{ position: "relative", width, background: "var(--panel)", borderBottom: "1px solid var(--border)", cursor: "text" }}
            onPointerDown={(e) => {
              (e.currentTarget as Element).setPointerCapture(e.pointerId);
              const t = snapT(timeAt(e.clientX));
              setDrag({ kind: "select", t0: t });
            }}
            onPointerUp={(e) => {
              if (drag?.kind === "select" && !selection) {
                const t = timeAt(e.clientX);
                api.studioLocate(Math.round(t * sr)).catch(error);
              }
            }}
            title="Click to move the playhead · drag to select a range"
          >
            {loop && <div style={{ position: "absolute", top: 0, height: 6, left: (loop.start / sr) * pps, width: ((loop.end - loop.start) / sr) * pps, background: "var(--accent)", opacity: 0.8, borderRadius: 2 }} />}
            {ticks.map((t, i) => (
              <div key={i} style={{ position: "absolute", left: t.x, bottom: 0, height: t.major ? 12 : 6, width: 1, background: t.major ? "#4a5462" : "#2b323c" }}>
                {t.label && <span className="mono" style={{ position: "absolute", left: 3, bottom: 9, fontSize: 10, color: "var(--text-3)" }}>{t.label}</span>}
              </div>
            ))}
            {p.markers.map((m) => (
              <div key={m.id} title={m.label} style={{ position: "absolute", left: (m.position / sr) * pps, top: 0, bottom: 0, borderLeft: "1px dashed var(--warn)" }}>
                <span style={{ fontSize: 10, color: "var(--warn)", marginLeft: 3 }}>{m.label}</span>
              </div>
            ))}
          </div>
        </div>

        {/* tracks */}
        {p.tracks.map((t, ti) => {
          const meters = status?.tracks[ti];
          const groupClips = new Set(t.takeGroups.flatMap((g) => g.takes));
          return (
            <div key={t.id} style={{ display: "flex", height: t.height, borderBottom: "1px solid var(--border)" }}>
              <div
                style={{ position: "sticky", left: 0, zIndex: 3, width: HEADER_W, flex: "none", background: selectedTrack === t.id ? "var(--panel-2)" : "var(--panel)", borderRight: "1px solid var(--border)", padding: "6px 8px", display: "flex", gap: 8 }}
                onPointerDown={() => onSelectTrack(t.id)}
              >
                <div style={{ width: 3, borderRadius: 2, background: t.color }} />
                <div className="col grow" style={{ gap: 4 }}>
                  <input
                    className="input"
                    defaultValue={t.name}
                    key={t.name}
                    aria-label="Track name"
                    style={{ height: 24, background: "transparent", border: "1px solid transparent", padding: "0 4px", fontWeight: 600 }}
                    onBlur={(e) => e.target.value.trim() && e.target.value !== t.name && void edit({ op: "updateTrack", track: t.id, update: { name: e.target.value } })}
                    onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                  />
                  <div className="row" style={{ gap: 4 }}>
                    <button className="toggle-chip toggle-chip--mute" aria-pressed={t.mute} title="Mute" onClick={() => void edit({ op: "updateTrack", track: t.id, update: { mute: !t.mute } })}>M</button>
                    <button className="toggle-chip toggle-chip--solo" aria-pressed={t.solo} title="Solo" onClick={() => void edit({ op: "updateTrack", track: t.id, update: { solo: !t.solo } })}>S</button>
                    <button className="toggle-chip toggle-chip--arm" aria-pressed={t.armed} title="Arm for recording" onClick={() => void edit({ op: "updateTrack", track: t.id, update: { armed: !t.armed } })}>R</button>
                    <button
                      className="toggle-chip toggle-chip--mon"
                      aria-pressed={t.monitor !== "off"}
                      title={`Input monitoring: ${t.monitor === "armed" ? "when armed" : t.monitor} (use headphones)`}
                      onClick={() => void edit({ op: "updateTrack", track: t.id, update: { monitor: t.monitor === "off" ? "armed" : t.monitor === "armed" ? "on" : "off" } })}
                    >
                      {t.monitor === "on" ? "MON" : t.monitor === "armed" ? "mon" : "—"}
                    </button>
                    <select
                      className="select"
                      aria-label="Record input"
                      style={{ height: 22, fontSize: 11, padding: "0 4px" }}
                      value={`${t.inputChannel}:${t.inputStereo ? 2 : 1}`}
                      onChange={(e) => {
                        const [ch, n] = e.target.value.split(":").map(Number);
                        void edit({ op: "updateTrack", track: t.id, update: { inputChannel: ch, inputStereo: n === 2 } });
                      }}
                    >
                      {Array.from({ length: Math.max(1, status?.backend.inputChannels ?? 1) }, (_, i) => (
                        <option key={`m${i}`} value={`${i}:1`}>In {i + 1}</option>
                      ))}
                      {Array.from({ length: Math.max(0, (status?.backend.inputChannels ?? 1) - 1) }, (_, i) => (
                        <option key={`s${i}`} value={`${i}:2`}>In {i + 1}+{i + 2}</option>
                      ))}
                    </select>
                  </div>
                  <div className="row" style={{ gap: 6 }}>
                    <input
                      className="slider"
                      type="range"
                      min={-1}
                      max={1}
                      step={0.01}
                      aria-label="Pan"
                      title={`Pan ${t.pan.toFixed(2)}`}
                      defaultValue={t.pan}
                      key={`pan${t.pan}`}
                      style={{ width: 70 }}
                      onInput={(e) => change({ op: "updateTrack", track: t.id, update: { pan: Number((e.target as HTMLInputElement).value) } }, false)}
                      onChange={() => undefined}
                      onPointerUp={(e) => change({ op: "updateTrack", track: t.id, update: { pan: Number((e.target as HTMLInputElement).value) } }, true)}
                      onDoubleClick={() => void edit({ op: "updateTrack", track: t.id, update: { pan: 0 } })}
                    />
                    <span className="mono faint" style={{ fontSize: 10 }}>{t.gainDb.toFixed(1)} dB</span>
                  </div>
                </div>
                {meters && <Meter values={meters} length={t.height - 14} thickness={4} label={`${t.name} level`} />}
              </div>
              <div
                style={{ position: "relative", width, background: ti % 2 ? "rgba(255,255,255,0.012)" : "transparent" }}
                onPointerDown={(e) => {
                  if (e.target === e.currentTarget) {
                    onSelectClip(null, null);
                    onSelectTrack(t.id);
                    (e.currentTarget as Element).setPointerCapture(e.pointerId);
                    setDrag({ kind: "select", t0: snapT(timeAt(e.clientX)) });
                  }
                }}
              >
                {t.clips
                  .filter((c) => !groupClips.has(c.id))
                  .map((c) => {
                    const dragging = drag && "clip" in drag && drag.clip.id === c.id ? drag : null;
                    if (dragging?.kind === "move" && dragging.targetTrack !== t.id) return null;
                    const start = dragging?.kind === "move" ? dragging.pos : dragging?.kind === "trimStart" ? dragging.start : c.position / sr;
                    const end = dragging?.kind === "move" ? dragging.pos + c.length / sr : dragging?.kind === "trimEnd" ? dragging.end : dragging?.kind === "trimStart" ? (c.position + c.length) / sr : (c.position + c.length) / sr;
                    const srcStart = c.sourceOffset + (dragging?.kind === "trimStart" ? Math.round((start - c.position / sr) * sr) : 0);
                    return (
                      <ClipBox
                        key={c.id}
                        clip={c}
                        color={t.color}
                        left={start * pps}
                        width={Math.max(4, (end - start) * pps)}
                        height={t.height - 6}
                        srcStart={srcStart}
                        srcEnd={srcStart + Math.round((end - start) * sr)}
                        selected={selectedClip === c.id}
                        onDown={(e, kind) => {
                          e.stopPropagation();
                          onSelectClip(c.id, t.id);
                          (e.currentTarget as Element).setPointerCapture(e.pointerId);
                          if (kind === "move") setDrag({ kind, clip: c, track: t.id, x0: e.clientX, y0: e.clientY, pos: c.position / sr, targetTrack: t.id });
                          else setDrag({ kind, clip: c, x0: e.clientX, start: c.position / sr, end: (c.position + c.length) / sr });
                        }}
                      />
                    );
                  })}
                {/* clips dragged in from another track */}
                {drag?.kind === "move" && drag.targetTrack === t.id && drag.track !== t.id && (
                  <div style={{ position: "absolute", top: 3, left: drag.pos * pps, width: (drag.clip.length / sr) * pps, height: t.height - 6, border: "1px dashed var(--accent)", borderRadius: 4 }} />
                )}
                {/* comp segments of take groups */}
                {t.takeGroups.map((g) =>
                  g.comp.map((s, si) => {
                    const c = t.clips.find((x) => x.id === s.take);
                    if (!c) return null;
                    const takeNo = g.takes.indexOf(s.take) + 1;
                    const left = (s.start / sr) * pps;
                    const w = Math.max(4, ((s.end - s.start) / sr) * pps);
                    const so = c.sourceOffset + (s.start - c.position);
                    return (
                      <div
                        key={`${g.id}-${si}`}
                        onPointerDown={(e) => {
                          e.stopPropagation();
                          onSelectClip(c.id, t.id);
                        }}
                        title={`Take ${takeNo} of ${g.takes.length} — choose takes in the Takes panel`}
                        style={{ position: "absolute", top: 3, left, width: w, height: t.height - 6, background: "rgba(124,92,255,0.18)", border: `1px solid ${g.takes.includes(selectedClip ?? "") ? "var(--accent)" : "rgba(124,92,255,0.6)"}`, borderRadius: 4, overflow: "hidden" }}
                      >
                        <div style={{ fontSize: 10, padding: "1px 4px", color: "#d5ccff", position: "absolute", zIndex: 1 }}>Take {takeNo}</div>
                        <ClipWave assetId={c.assetId} start={so} end={so + (s.end - s.start)} width={w} height={t.height - 8} color="rgba(190,175,255,0.75)" />
                      </div>
                    );
                  }),
                )}
                {recordingSince != null && t.armed && playhead > recordingSince && (
                  <div style={{ position: "absolute", top: 3, left: recordingSince * pps, width: (playhead - recordingSince) * pps, height: t.height - 6, background: "rgba(255,59,78,0.22)", border: "1px solid var(--rec)", borderRadius: 4 }} />
                )}
              </div>
            </div>
          );
        })}

        {/* overlays: selection, loop, playhead */}
        <div style={{ position: "absolute", top: RULER_H, bottom: 0, left: HEADER_W, width, pointerEvents: "none" }}>
          {loop && <div style={{ position: "absolute", top: 0, bottom: 0, left: (loop.start / sr) * pps, width: ((loop.end - loop.start) / sr) * pps, background: "rgba(59,158,255,0.05)", borderLeft: "1px solid var(--accent-line)", borderRight: "1px solid var(--accent-line)" }} />}
          {selection && <div style={{ position: "absolute", top: 0, bottom: 0, left: selection[0] * pps, width: (selection[1] - selection[0]) * pps, background: "rgba(255,255,255,0.06)" }} />}
          <div style={{ position: "absolute", top: -RULER_H, bottom: 0, left: playhead * pps, width: 1.5, background: status?.recording ? "var(--rec)" : "var(--text)" }} />
        </div>
      </div>
    </div>
  );
}

function ClipBox({
  clip,
  color,
  left,
  width,
  height,
  srcStart,
  srcEnd,
  selected,
  onDown,
}: {
  clip: Clip;
  color: string;
  left: number;
  width: number;
  height: number;
  srcStart: number;
  srcEnd: number;
  selected: boolean;
  onDown: (e: React.PointerEvent, kind: "move" | "trimStart" | "trimEnd") => void;
}) {
  return (
    <div
      role="button"
      aria-label={`Clip ${clip.name}`}
      aria-pressed={selected}
      style={{
        position: "absolute",
        top: 3,
        left,
        width,
        height,
        background: `${color}22`,
        border: `1px solid ${selected ? "#e6edf3" : `${color}99`}`,
        borderRadius: 4,
        overflow: "hidden",
        cursor: "grab",
        opacity: clip.muted ? 0.4 : 1,
      }}
      onPointerDown={(e) => onDown(e, "move")}
    >
      <div style={{ fontSize: 10, padding: "1px 5px", color: "#e6edf3", position: "absolute", zIndex: 1, whiteSpace: "nowrap", textShadow: "0 1px 2px #000" }}>
        {clip.name}
        {clip.gainDb !== 0 ? ` (${clip.gainDb > 0 ? "+" : ""}${clip.gainDb.toFixed(1)} dB)` : ""}
      </div>
      <ClipWave assetId={clip.assetId} start={srcStart} end={srcEnd} width={width} height={height - 2} color={color} />
      <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: 6, cursor: "ew-resize" }} onPointerDown={(e) => { e.stopPropagation(); onDown(e, "trimStart"); }} />
      <div style={{ position: "absolute", right: 0, top: 0, bottom: 0, width: 6, cursor: "ew-resize" }} onPointerDown={(e) => { e.stopPropagation(); onDown(e, "trimEnd"); }} />
    </div>
  );
}
