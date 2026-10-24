import { useEffect, useRef } from "react";
import type { MeterValue } from "../api/types";
import { gainToDb } from "../lib/music";

const MIN_DB = -60;
const MAX_DB = 3;

const norm = (db: number) => Math.max(0, Math.min(1, (db - MIN_DB) / (MAX_DB - MIN_DB)));

/**
 * Peak/RMS meter drawn on a canvas. Peak bars use studio colours
 * (green < −12 dBFS < yellow < −3 dBFS < red); a peak-hold line decays
 * slowly and a clip box latches until clicked.
 */
export function Meter({
  values,
  orientation = "vertical",
  length = 120,
  thickness = 6,
  onResetClip,
  label,
}: {
  values: MeterValue[];
  orientation?: "vertical" | "horizontal";
  length?: number;
  thickness?: number;
  onResetClip?: () => void;
  label?: string;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const hold = useRef<{ v: number; t: number }[]>([]);
  const vertical = orientation === "vertical";
  const n = Math.max(1, values.length);
  const gap = 2;
  const clipSize = 6;
  const w = vertical ? n * thickness + (n - 1) * gap : length;
  const h = vertical ? length : n * thickness + (n - 1) * gap;

  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== w * dpr) {
      c.width = w * dpr;
      c.height = h * dpr;
    }
    const ctx = c.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const now = performance.now();
    const span = (vertical ? h : w) - clipSize - 2;
    values.forEach((m, i) => {
      const peak = norm(gainToDb(m.peak));
      const rms = norm(gainToDb(m.rms));
      const hv = hold.current[i];
      if (!hv || peak >= hv.v || now - hv.t > 1500) hold.current[i] = { v: peak, t: now };
      else hold.current[i] = { v: Math.max(peak, hv.v - 0.004), t: hv.t };
      const off = i * (thickness + gap);
      // background
      ctx.fillStyle = "#07090b";
      if (vertical) ctx.fillRect(off, clipSize + 2, thickness, span);
      else ctx.fillRect(0, off, span, thickness);
      // segments
      const segs: [number, number, string][] = [
        [0, norm(-12), "#2fd38a"],
        [norm(-12), norm(-3), "#f2c94c"],
        [norm(-3), 1, "#ff5c6c"],
      ];
      for (const [a, b, col] of segs) {
        if (peak <= a) continue;
        const top = Math.min(peak, b);
        ctx.fillStyle = col;
        ctx.globalAlpha = 0.85;
        if (vertical) ctx.fillRect(off, clipSize + 2 + span * (1 - top), thickness, span * (top - a));
        else ctx.fillRect(span * a, off, span * (top - a), thickness);
      }
      // rms overlay
      ctx.globalAlpha = 0.35;
      ctx.fillStyle = "#ffffff";
      if (vertical) ctx.fillRect(off + thickness / 2 - 1, clipSize + 2 + span * (1 - rms), 2, span * rms);
      else ctx.fillRect(0, off + thickness / 2 - 1, span * rms, 2);
      ctx.globalAlpha = 1;
      // peak hold
      const hp = hold.current[i]!.v;
      if (hp > 0.01) {
        ctx.fillStyle = hp > norm(-3) ? "#ff5c6c" : "#e6edf3";
        if (vertical) ctx.fillRect(off, clipSize + 2 + span * (1 - hp), thickness, 1.5);
        else ctx.fillRect(span * hp, off, 1.5, thickness);
      }
      // clip indicator
      ctx.fillStyle = m.clipped ? "#ff3b4e" : "#1b2027";
      if (vertical) ctx.fillRect(off, 0, thickness, clipSize);
      else ctx.fillRect(w - clipSize, off, clipSize, thickness);
    });
  }, [values, w, h, vertical, thickness]);

  const clipped = values.some((v) => v.clipped);
  const peakDb = Math.max(...values.map((v) => gainToDb(v.peak)), -120);
  return (
    <canvas
      ref={canvas}
      className="meter"
      style={{ width: w, height: h, cursor: onResetClip ? "pointer" : undefined }}
      role="meter"
      aria-label={`${label ?? "Level"}: ${peakDb <= -119 ? "silence" : `${peakDb.toFixed(0)} dBFS peak`}${clipped ? ", clipped" : ""}`}
      aria-valuemin={MIN_DB}
      aria-valuemax={MAX_DB}
      aria-valuenow={Math.max(MIN_DB, peakDb)}
      title={clipped ? "Clipped — click to reset" : undefined}
      onClick={onResetClip}
    />
  );
}
