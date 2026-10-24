import { useCallback, useEffect, useRef, useState } from "react";
import type { DetectedNote, PitchFrame, TargetNote } from "../api/types";
import { formatCents, formatTime, hzToMidi, noteName } from "../lib/music";

export interface PitchViewProps {
  frames: PitchFrame[];
  duration: number;
  /** Interleaved [min,max] waveform pairs across the full duration. */
  peaks?: number[];
  targets?: TargetNote[];
  notes?: DetectedNote[];
  /** Second contour to overlay (e.g. a compared take), drawn in grey. */
  compare?: PitchFrame[];
  cursor?: number | null;
  selection?: [number, number] | null;
  onSelect?: (sel: [number, number] | null) => void;
  onSeek?: (t: number) => void;
  height?: number;
  toleranceCents?: number;
  /** Fixed MIDI range; otherwise derived from the data. */
  midiRange?: [number, number];
}

const PAD_L = 44;
const PAD_R = 8;
const WAVE_H = 46;

function rangeOf(frames: PitchFrame[], targets?: TargetNote[], fixed?: [number, number]): [number, number] {
  if (fixed) return fixed;
  const ms: number[] = [];
  for (const f of frames) if (f.hz) ms.push(hzToMidi(f.hz));
  for (const t of targets ?? []) ms.push(t.midi);
  if (!ms.length) return [48, 66];
  ms.sort((a, b) => a - b);
  const lo = ms[Math.floor(ms.length * 0.02)]!;
  const hi = ms[Math.floor(ms.length * 0.98)]!;
  const mid = (lo + hi) / 2;
  const span = Math.max(12, hi - lo + 6);
  return [Math.floor(mid - span / 2), Math.ceil(mid + span / 2)];
}

/**
 * Time-aligned waveform + pitch contour. Pitch line opacity follows the
 * measurement confidence; unvoiced or low-confidence regions are left blank
 * rather than guessed.
 */
export function PitchView(props: PitchViewProps) {
  const { frames, duration, peaks, targets, notes, compare, cursor, selection, onSelect, onSeek, height = 300, toleranceCents = 25, midiRange } = props;
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(800);
  const [hover, setHover] = useState<{ x: number; y: number; t: number; text: string } | null>(null);
  const drag = useRef<number | null>(null);
  const [range] = [rangeOf(frames, targets, midiRange)];
  const plotH = height - WAVE_H - 18;

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver((e) => setWidth(Math.max(200, Math.floor(e[0]!.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const xOf = useCallback((t: number) => PAD_L + (t / Math.max(duration, 1e-3)) * (width - PAD_L - PAD_R), [duration, width]);
  const tOf = useCallback((x: number) => Math.max(0, Math.min(duration, ((x - PAD_L) / (width - PAD_L - PAD_R)) * duration)), [duration, width]);
  const yOf = useCallback((m: number) => 8 + (1 - (m - range[0]) / (range[1] - range[0])) * (plotH - 8), [range, plotH]);

  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = width * dpr;
    c.height = height * dpr;
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.font = "10px Segoe UI, sans-serif";

    // note grid
    for (let m = Math.ceil(range[0]); m <= range[1]; m++) {
      const y = yOf(m);
      const isC = m % 12 === 0;
      const natural = [0, 2, 4, 5, 7, 9, 11].includes(((m % 12) + 12) % 12);
      ctx.fillStyle = natural ? "rgba(255,255,255,0.025)" : "rgba(0,0,0,0.18)";
      const h = (plotH - 8) / (range[1] - range[0]);
      ctx.fillRect(PAD_L, y - h / 2, width - PAD_L - PAD_R, h);
      ctx.strokeStyle = isC ? "rgba(255,255,255,0.14)" : "rgba(255,255,255,0.05)";
      ctx.beginPath();
      ctx.moveTo(PAD_L, y + h / 2);
      ctx.lineTo(width - PAD_R, y + h / 2);
      ctx.stroke();
      if (natural || isC) {
        ctx.fillStyle = isC ? "#a9b4c0" : "#6f7b88";
        ctx.fillText(noteName(m), 6, y + 3);
      }
    }

    // selection
    if (selection) {
      ctx.fillStyle = "rgba(59,158,255,0.10)";
      ctx.fillRect(xOf(selection[0]), 0, xOf(selection[1]) - xOf(selection[0]), height - 18);
    }

    // targets with tolerance band
    for (const t of targets ?? []) {
      const x0 = xOf(t.start);
      const x1 = xOf(t.start + t.duration);
      const band = toleranceCents / 100;
      ctx.fillStyle = "rgba(59,158,255,0.12)";
      ctx.fillRect(x0, yOf(t.midi + band), x1 - x0, yOf(t.midi - band) - yOf(t.midi + band));
      ctx.strokeStyle = "rgba(92,176,255,0.85)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x0, yOf(t.midi));
      ctx.lineTo(x1, yOf(t.midi));
      ctx.stroke();
    }

    // detected notes (subtle)
    for (const n of notes ?? []) {
      ctx.fillStyle = "rgba(255,255,255,0.05)";
      ctx.fillRect(xOf(n.start), yOf(n.midi + 0.35), xOf(n.end) - xOf(n.start), yOf(n.midi - 0.35) - yOf(n.midi + 0.35));
    }

    const drawContour = (fr: PitchFrame[], color: string, width_: number, useConf: boolean) => {
      ctx.lineWidth = width_;
      ctx.lineCap = "round";
      let prev: { x: number; y: number; t: number } | null = null;
      for (const f of fr) {
        if (!f.hz) {
          prev = null;
          continue;
        }
        const x = xOf(f.time);
        const y = yOf(hzToMidi(f.hz));
        if (prev && f.time - prev.t < 0.05) {
          ctx.strokeStyle = color;
          ctx.globalAlpha = useConf ? Math.max(0.25, Math.min(1, (f.confidence - 0.5) * 2.2)) : 0.6;
          ctx.beginPath();
          ctx.moveTo(prev.x, prev.y);
          ctx.lineTo(x, y);
          ctx.stroke();
        }
        prev = { x, y, t: f.time };
      }
      ctx.globalAlpha = 1;
    };
    if (compare) drawContour(compare, "#8b96a3", 1.5, false);
    drawContour(frames, "#5cb0ff", 2.2, true);

    // waveform strip
    const wy = height - WAVE_H - 14;
    ctx.fillStyle = "#07090b";
    ctx.fillRect(PAD_L, wy, width - PAD_L - PAD_R, WAVE_H);
    if (peaks && peaks.length >= 2) {
      const n = peaks.length / 2;
      const mid = wy + WAVE_H / 2;
      ctx.fillStyle = "#3b6f9e";
      for (let i = 0; i < n; i++) {
        const x = PAD_L + (i / n) * (width - PAD_L - PAD_R);
        const mn = peaks[2 * i]!;
        const mx = peaks[2 * i + 1]!;
        ctx.fillRect(x, mid - mx * (WAVE_H / 2), Math.max(1, (width - PAD_L - PAD_R) / n), Math.max(1, (mx - mn) * (WAVE_H / 2)));
      }
    }
    // time axis
    ctx.fillStyle = "#6f7b88";
    const step = duration > 60 ? 10 : duration > 20 ? 5 : duration > 6 ? 1 : 0.5;
    for (let t = 0; t <= duration + 1e-6; t += step) {
      const x = xOf(t);
      ctx.fillRect(x, height - 14, 1, 4);
      ctx.fillText(step < 1 ? t.toFixed(1) : formatTime(t), x + 2, height - 3);
    }
    // cursor
    if (cursor != null && cursor >= 0) {
      ctx.fillStyle = "#e6edf3";
      ctx.fillRect(xOf(cursor), 0, 1.5, height - 14);
    }
  }, [frames, compare, peaks, targets, notes, selection, cursor, width, height, range, xOf, yOf, plotH, duration, toleranceCents]);

  const describe = (x: number, y: number) => {
    const t = tOf(x);
    let best: PitchFrame | null = null;
    for (const f of frames) if (!best || Math.abs(f.time - t) < Math.abs(best.time - t)) best = f;
    let text = formatTime(t, true);
    if (best?.hz) {
      const m = hzToMidi(best.hz);
      text += ` · ${noteName(m)} ${formatCents((m - Math.round(m)) * 100)} · ${best.hz.toFixed(1)} Hz · confidence ${(best.confidence * 100).toFixed(0)}%`;
    } else if (best) {
      text += best.confidence > 0 ? ` · not voiced (confidence ${(best.confidence * 100).toFixed(0)}%)` : " · silence";
    }
    setHover({ x, y, t, text });
  };

  return (
    <div ref={wrap} style={{ position: "relative", width: "100%" }}>
      <canvas
        ref={canvas}
        style={{ width: "100%", height, display: "block", cursor: onSelect ? "crosshair" : onSeek ? "pointer" : "default" }}
        role="img"
        aria-label={`Pitch contour over ${duration.toFixed(1)} seconds${targets?.length ? ` with ${targets.length} target notes` : ""}`}
        onMouseMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          const x = e.clientX - r.left;
          describe(x, e.clientY - r.top);
          if (drag.current != null && onSelect) {
            const a = drag.current;
            const b = tOf(x);
            onSelect(Math.abs(b - a) > 0.05 ? [Math.min(a, b), Math.max(a, b)] : null);
          }
        }}
        onMouseLeave={() => setHover(null)}
        onMouseDown={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          drag.current = tOf(e.clientX - r.left);
        }}
        onMouseUp={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          const t = tOf(e.clientX - r.left);
          if (drag.current != null && Math.abs(t - drag.current) < 0.05) {
            onSeek?.(t);
            onSelect?.(null);
          }
          drag.current = null;
        }}
      />
      {hover && (
        <div
          className="mono"
          style={{
            position: "absolute",
            left: Math.min(hover.x + 12, width - 330),
            top: Math.max(4, hover.y - 30),
            background: "var(--raised)",
            border: "1px solid var(--border-strong)",
            borderRadius: 4,
            padding: "3px 7px",
            fontSize: 11,
            pointerEvents: "none",
            whiteSpace: "nowrap",
          }}
        >
          {hover.text}
        </div>
      )}
    </div>
  );
}
