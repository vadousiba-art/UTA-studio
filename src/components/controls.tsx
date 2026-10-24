import { useEffect, useRef, useState } from "react";
import { api } from "../api/client";
import type { JobInfo, ParamInfo } from "../api/types";
import { clamp } from "../lib/music";

/** Map a parameter value to 0..1 for display/drag, honouring log scales. */
export function toUnit(p: ParamInfo, v: number): number {
  if (p.scale.type === "log") return Math.log(v / p.min) / Math.log(p.max / p.min);
  return (v - p.min) / (p.max - p.min);
}
export function fromUnit(p: ParamInfo, u: number): number {
  u = clamp(u, 0, 1);
  if (p.scale.type === "log") return p.min * Math.pow(p.max / p.min, u);
  const v = p.min + u * (p.max - p.min);
  return p.scale.type === "toggle" || p.scale.type === "choice" ? Math.round(v) : v;
}

export function formatParam(p: ParamInfo, v: number): string {
  if (p.scale.type === "toggle") return v >= 0.5 ? "On" : "Off";
  if (p.scale.type === "choice") return p.scale.labels[Math.round(v)] ?? String(v);
  const digits = Math.abs(v) >= 100 ? 0 : Math.abs(v) >= 10 ? 1 : 2;
  if (p.unit === "Hz" && v >= 1000) return `${(v / 1000).toFixed(v >= 10000 ? 1 : 2)} kHz`;
  return `${v.toFixed(digits)}${p.unit ? ` ${p.unit}` : ""}`;
}

/**
 * Rotary knob. Drag vertically (Shift for fine), arrow keys to step,
 * double-click to reset. Calls `onChange(v, final)`: `final` is false while
 * dragging and true on release, so the caller can group one undo step.
 */
export function Knob({ param, value, onChange, size = 44 }: { param: ParamInfo; value: number; onChange: (v: number, final: boolean) => void; size?: number }) {
  const start = useRef<{ y: number; u: number } | null>(null);
  const [local, setLocal] = useState(value);
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setLocal(value);
  }, [value]);
  const u = clamp(toUnit(param, local), 0, 1);
  const angle = -135 + u * 270;
  const r = size / 2 - 4;
  const arc = (a: number) => {
    const rad = ((a - 90) * Math.PI) / 180;
    return [size / 2 + r * Math.cos(rad), size / 2 + r * Math.sin(rad)];
  };
  const [sx, sy] = arc(-135);
  const [ex, ey] = arc(angle);
  const large = angle + 135 > 180 ? 1 : 0;
  const commit = (v: number, final: boolean) => {
    setLocal(v);
    onChange(v, final);
  };
  return (
    <div className="col" style={{ alignItems: "center", gap: 2, width: size + 26 }}>
      <svg
        width={size}
        height={size}
        role="slider"
        tabIndex={0}
        aria-label={param.name}
        aria-valuemin={param.min}
        aria-valuemax={param.max}
        aria-valuenow={local}
        aria-valuetext={formatParam(param, local)}
        style={{ cursor: "ns-resize", touchAction: "none" }}
        onPointerDown={(e) => {
          (e.target as Element).setPointerCapture(e.pointerId);
          start.current = { y: e.clientY, u };
          dragging.current = true;
        }}
        onPointerMove={(e) => {
          if (!start.current) return;
          const dy = start.current.y - e.clientY;
          const nu = clamp(start.current.u + dy / (e.shiftKey ? 600 : 150), 0, 1);
          commit(fromUnit(param, nu), false);
        }}
        onPointerUp={() => {
          if (start.current) onChange(local, true);
          start.current = null;
          dragging.current = false;
        }}
        onDoubleClick={() => commit(param.default, true)}
        onKeyDown={(e) => {
          const step = e.shiftKey ? 0.002 : 0.02;
          if (e.key === "ArrowUp" || e.key === "ArrowRight") commit(fromUnit(param, u + step), true);
          else if (e.key === "ArrowDown" || e.key === "ArrowLeft") commit(fromUnit(param, u - step), true);
          else return;
          e.preventDefault();
        }}
      >
        <circle cx={size / 2} cy={size / 2} r={r} fill="#0b0e12" stroke="#2a313c" strokeWidth={3} />
        {u > 0.002 && <path d={`M ${sx} ${sy} A ${r} ${r} 0 ${large} 1 ${ex} ${ey}`} fill="none" stroke="#3b9eff" strokeWidth={3} strokeLinecap="round" />}
        <line x1={size / 2} y1={size / 2} x2={ex} y2={ey} stroke="#e6edf3" strokeWidth={2} strokeLinecap="round" />
      </svg>
      <div className="faint" style={{ fontSize: 10, textAlign: "center", lineHeight: 1.2 }}>{param.name}</div>
      <div className="mono" style={{ fontSize: 10.5 }}>{formatParam(param, local)}</div>
    </div>
  );
}

/** Vertical fader in dB (−60..+6) with an audio taper. */
export function Fader({ db, onChange, height = 150, label }: { db: number; onChange: (db: number, final: boolean) => void; height?: number; label: string }) {
  const MIN = -60;
  const MAX = 6;
  const toPos = (d: number) => Math.pow(clamp((d - MIN) / (MAX - MIN), 0, 1), 1.6);
  const fromPos = (p: number) => (p <= 0.001 ? -96 : MIN + Math.pow(clamp(p, 0, 1), 1 / 1.6) * (MAX - MIN));
  const [local, setLocal] = useState(db);
  const dragging = useRef(false);
  const track = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!dragging.current) setLocal(db);
  }, [db]);
  const p = toPos(local);
  const set = (clientY: number, final: boolean) => {
    const r = track.current!.getBoundingClientRect();
    const pos = 1 - (clientY - r.top) / r.height;
    const v = Math.round(fromPos(pos) * 10) / 10;
    setLocal(v);
    onChange(v, final);
  };
  return (
    <div
      ref={track}
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={-96}
      aria-valuemax={MAX}
      aria-valuenow={local}
      aria-valuetext={`${local <= -95 ? "−∞" : local.toFixed(1)} dB`}
      style={{ position: "relative", width: 28, height, cursor: "ns-resize", touchAction: "none" }}
      onPointerDown={(e) => {
        (e.currentTarget as Element).setPointerCapture(e.pointerId);
        dragging.current = true;
        set(e.clientY, false);
      }}
      onPointerMove={(e) => dragging.current && set(e.clientY, false)}
      onPointerUp={(e) => {
        dragging.current = false;
        set(e.clientY, true);
      }}
      onDoubleClick={() => {
        setLocal(0);
        onChange(0, true);
      }}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 0.1 : 1;
        if (e.key === "ArrowUp") onChange(Math.min(MAX, local + step), true);
        else if (e.key === "ArrowDown") onChange(Math.max(-96, local - step), true);
        else return;
        e.preventDefault();
      }}
    >
      <div style={{ position: "absolute", left: 12, top: 0, bottom: 0, width: 4, background: "#07090b", borderRadius: 2 }} />
      <div style={{ position: "absolute", left: 0, right: 0, top: `${(1 - toPos(0)) * 100}%`, height: 1, background: "#3a4350" }} />
      <div
        style={{
          position: "absolute",
          left: 2,
          width: 24,
          height: 14,
          top: `calc(${(1 - p) * 100}% - 7px)`,
          background: "linear-gradient(#d6dde5, #9aa6b2)",
          borderRadius: 3,
          boxShadow: "0 1px 3px rgba(0,0,0,.6)",
        }}
      />
    </div>
  );
}

/** Poll a background job until it finishes. */
export function useJob(id: number | null, onDone?: (j: JobInfo) => void) {
  const [job, setJob] = useState<JobInfo | null>(null);
  const done = useRef(onDone);
  done.current = onDone;
  useEffect(() => {
    if (id == null) {
      setJob(null);
      return;
    }
    let alive = true;
    const t = window.setInterval(async () => {
      try {
        const all = await api.jobs();
        const j = all.find((x) => x.id === id) ?? null;
        if (!alive) return;
        setJob(j);
        if (j && j.state !== "queued" && j.state !== "running") {
          window.clearInterval(t);
          done.current?.(j);
        }
      } catch {
        /* ignore transient errors */
      }
    }, 200);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [id]);
  return job;
}
