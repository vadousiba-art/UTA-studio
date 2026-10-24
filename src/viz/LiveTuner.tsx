import { useEffect, useRef, useState } from "react";
import type { LiveFrame } from "../api/types";
import { hzToMidi, noteName } from "../lib/music";

/**
 * Live pitch display: nearest note, cents needle and a scrolling 4-second
 * contour. Shows "—" instead of guessing when the input isn't confidently
 * voiced.
 */
export function LiveTuner({ framesRef, target }: { framesRef: React.MutableRefObject<LiveFrame[]>; target?: number | null }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [current, setCurrent] = useState<{ midi: number; conf: number } | null>(null);

  useEffect(() => {
    let raf = 0;
    let last = 0;
    const draw = (now: number) => {
      raf = requestAnimationFrame(draw);
      if (now - last < 33) return;
      last = now;
      const frames = framesRef.current;
      const latest = frames[frames.length - 1];
      const recentVoiced = latest && latest.hz && performance.now() ? latest : null;
      setCurrent(recentVoiced?.hz ? { midi: hzToMidi(recentVoiced.hz), conf: recentVoiced.confidence } : null);
      const c = canvas.current;
      if (!c) return;
      const w = c.clientWidth;
      const h = c.clientHeight;
      const dpr = window.devicePixelRatio || 1;
      if (c.width !== w * dpr) {
        c.width = w * dpr;
        c.height = h * dpr;
      }
      const ctx = c.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      if (!frames.length) return;
      const tEnd = frames[frames.length - 1]!.time;
      const span = 4;
      const voiced = frames.filter((f) => f.hz && f.time > tEnd - span).map((f) => hzToMidi(f.hz!));
      const center = target ?? (voiced.length ? voiced[voiced.length - 1]! : 57);
      const lo = center - 6;
      const hi = center + 6;
      const y = (m: number) => h - ((m - lo) / (hi - lo)) * h;
      for (let m = Math.ceil(lo); m <= hi; m++) {
        ctx.fillStyle = m === Math.round(center) ? "rgba(59,158,255,0.18)" : "rgba(255,255,255,0.04)";
        ctx.fillRect(0, y(m) - 0.5, w, 1);
      }
      if (target != null) {
        ctx.fillStyle = "rgba(59,158,255,0.12)";
        ctx.fillRect(0, y(target + 0.25), w, y(target - 0.25) - y(target + 0.25));
      }
      ctx.strokeStyle = "#5cb0ff";
      ctx.lineWidth = 2;
      let prev: { x: number; y: number; t: number } | null = null;
      for (const f of frames) {
        if (f.time < tEnd - span) continue;
        if (!f.hz) {
          prev = null;
          continue;
        }
        const x = w - ((tEnd - f.time) / span) * w;
        const yy = y(hzToMidi(f.hz));
        if (prev && f.time - prev.t < 0.06) {
          ctx.globalAlpha = Math.max(0.3, Math.min(1, (f.confidence - 0.5) * 2.2));
          ctx.beginPath();
          ctx.moveTo(prev.x, prev.y);
          ctx.lineTo(x, yy);
          ctx.stroke();
        }
        prev = { x, y: yy, t: f.time };
      }
      ctx.globalAlpha = 1;
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [framesRef, target]);

  const ref = target ?? (current ? Math.round(current.midi) : null);
  const cents = current && ref != null ? (current.midi - ref) * 100 : null;
  const inTune = cents != null && Math.abs(cents) <= 15;
  return (
    <div className="row" style={{ gap: 16, alignItems: "stretch" }}>
      <div style={{ width: 150, textAlign: "center" }} aria-live="polite">
        <div style={{ fontSize: 40, fontWeight: 700, lineHeight: 1.1, color: current ? (inTune ? "var(--ok)" : "var(--text)") : "var(--text-3)" }}>
          {current ? noteName(current.midi) : "—"}
        </div>
        <div className="mono muted" style={{ fontSize: 13 }}>
          {cents != null ? `${cents > 0 ? "+" : ""}${cents.toFixed(0)}¢${target != null ? ` vs ${noteName(target)}` : ""}` : "listening…"}
        </div>
        <div style={{ position: "relative", height: 10, marginTop: 10, background: "var(--bg-sunken)", borderRadius: 5 }}>
          <div style={{ position: "absolute", left: "50%", top: 0, bottom: 0, width: 1, background: "var(--text-3)" }} />
          {cents != null && (
            <div
              style={{
                position: "absolute",
                top: 1,
                width: 8,
                height: 8,
                borderRadius: 4,
                background: inTune ? "var(--ok)" : "var(--accent)",
                left: `calc(${50 + Math.max(-50, Math.min(50, cents))}% - 4px)`,
                transition: "left 60ms linear",
              }}
            />
          )}
        </div>
        <div className="faint" style={{ fontSize: 10, marginTop: 4 }}>
          {current ? `confidence ${(current.conf * 100).toFixed(0)}%` : "no clear pitch"}
        </div>
      </div>
      <canvas ref={canvas} style={{ flex: 1, height: 120, background: "var(--bg-sunken)", borderRadius: 6 }} aria-hidden />
    </div>
  );
}
