import { ArrowDown, ArrowUp, Power, Save, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import type { BuiltinKind, EffectCatalogEntry, EffectSlot, MeterValue, PresetEntry, StudioState } from "../../api/types";
import { Fader, Knob } from "../../components/controls";
import { Button, Modal, Notice } from "../../components/ui";
import { useApp } from "../../state/app";
import { Meter } from "../../viz/Meter";
import { useStudioActions } from "./useStudio";

export interface SlotRef {
  track: string | null;
  slot: string;
}

let catalogCache: EffectCatalogEntry[] | null = null;
export function useCatalog() {
  const [cat, setCat] = useState<EffectCatalogEntry[]>(catalogCache ?? []);
  useEffect(() => {
    if (catalogCache) return;
    api.effectCatalog().then((c) => {
      catalogCache = c;
      setCat(c);
    });
  }, []);
  return cat;
}

function ChainList({ track, effects, selected, onSelect }: { track: string | null; effects: EffectSlot[]; selected: SlotRef | null; onSelect: (s: SlotRef | null) => void }) {
  const { edit } = useStudioActions();
  const catalog = useCatalog();
  return (
    <div className="col" style={{ gap: 3, width: "100%" }}>
      {effects.map((e, i) => {
        const name = e.effect.type === "builtin" ? catalog.find((c) => c.id === (e.effect as { kind: string }).kind)?.name ?? e.effect.kind : e.effect.plugin.name;
        const sel = selected?.slot === e.id;
        return (
          <div
            key={e.id}
            className="row"
            style={{ gap: 2, padding: "2px 4px", borderRadius: 4, background: sel ? "var(--accent-dim)" : "var(--raised)", border: `1px solid ${sel ? "var(--accent-line)" : "var(--border)"}`, fontSize: 11, cursor: "pointer" }}
            onClick={() => onSelect({ track, slot: e.id })}
          >
            <button
              className="btn btn--ghost btn--sm btn--icon"
              aria-label={e.bypass ? `Enable ${name}` : `Bypass ${name}`}
              title={e.bypass ? "Bypassed — click to enable" : "Click to bypass"}
              style={{ color: e.bypass ? "var(--text-3)" : "var(--ok)", width: 18, height: 18 }}
              onClick={(ev) => {
                ev.stopPropagation();
                void edit({ op: "setEffectBypass", track, slot: e.id, bypass: !e.bypass });
              }}
            >
              <Power size={11} />
            </button>
            <span className="grow" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", opacity: e.bypass ? 0.5 : 1 }}>{name}</span>
            {i > 0 && (
              <button className="btn btn--ghost btn--sm btn--icon" style={{ width: 16, height: 16 }} aria-label="Move up" onClick={(ev) => { ev.stopPropagation(); void edit({ op: "moveEffect", track, slot: e.id, index: i - 1 }); }}>
                <ArrowUp size={10} />
              </button>
            )}
            {i < effects.length - 1 && (
              <button className="btn btn--ghost btn--sm btn--icon" style={{ width: 16, height: 16 }} aria-label="Move down" onClick={(ev) => { ev.stopPropagation(); void edit({ op: "moveEffect", track, slot: e.id, index: i + 1 }); }}>
                <ArrowDown size={10} />
              </button>
            )}
            <button
              className="btn btn--ghost btn--sm btn--icon"
              style={{ width: 16, height: 16 }}
              aria-label={`Remove ${name}`}
              onClick={(ev) => {
                ev.stopPropagation();
                if (sel) onSelect(null);
                void edit({ op: "removeEffect", track, slot: e.id });
              }}
            >
              <X size={10} />
            </button>
          </div>
        );
      })}
      <select
        className="select"
        aria-label="Add effect"
        value=""
        style={{ height: 22, fontSize: 11, padding: "0 4px" }}
        onChange={async (e) => {
          const v = e.target.value;
          if (!v) return;
          const st = await edit({ op: "insertEffect", track, effect: v as BuiltinKind, index: null });
          const chain = track ? st?.project.tracks.find((t) => t.id === track)?.effects : st?.project.master.effects;
          const last = chain?.[chain.length - 1];
          if (last) onSelect({ track, slot: last.id });
        }}
      >
        <option value="">+ Add effect…</option>
        {catalog.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
      </select>
    </div>
  );
}

export function Mixer({ state, selected, onSelect }: { state: StudioState; selected: SlotRef | null; onSelect: (s: SlotRef | null) => void }) {
  const status = useApp((s) => s.status);
  const { edit, change } = useStudioActions();
  const p = state.project;
  const strip = (key: string, name: string, color: string, meters: [MeterValue, MeterValue] | undefined, gainDb: number, onGain: (db: number, final: boolean) => void, extra: React.ReactNode, chain: React.ReactNode) => (
    <div key={key} className="col" style={{ width: 132, flex: "none", padding: 8, borderRight: "1px solid var(--border)", alignItems: "center", gap: 6, minHeight: "100%" }}>
      <div style={{ width: "100%", height: 3, background: color, borderRadius: 2 }} />
      <div style={{ fontWeight: 600, fontSize: 12, width: "100%", textAlign: "center", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</div>
      {chain}
      <div className="row" style={{ gap: 6, alignItems: "flex-end" }}>
        <Fader db={gainDb} onChange={onGain} height={120} label={`${name} level`} />
        <Meter values={meters ?? [{ peak: 0, rms: 0, clipped: false }, { peak: 0, rms: 0, clipped: false }]} length={120} thickness={5} onResetClip={() => void api.resetClips()} label={`${name} level`} />
      </div>
      <div className="mono" style={{ fontSize: 11 }}>{gainDb <= -95 ? "−∞" : `${gainDb > 0 ? "+" : ""}${gainDb.toFixed(1)}`} dB</div>
      {extra}
    </div>
  );
  return (
    <div style={{ display: "flex", alignItems: "flex-start", minHeight: "100%" }}>
      {p.tracks.map((t, i) =>
        strip(
          t.id,
          t.name,
          t.color,
          status?.tracks[i],
          t.gainDb,
          (db, final) => change({ op: "updateTrack", track: t.id, update: { gainDb: db } }, final),
          <div className="row" style={{ gap: 3 }}>
            <button className="toggle-chip toggle-chip--mute" aria-pressed={t.mute} onClick={() => void edit({ op: "updateTrack", track: t.id, update: { mute: !t.mute } })}>M</button>
            <button className="toggle-chip toggle-chip--solo" aria-pressed={t.solo} onClick={() => void edit({ op: "updateTrack", track: t.id, update: { solo: !t.solo } })}>S</button>
            <input
              className="slider"
              type="range"
              min={-1}
              max={1}
              step={0.01}
              aria-label={`${t.name} pan`}
              defaultValue={t.pan}
              key={`p${t.pan}`}
              style={{ width: 60 }}
              onInput={(e) => change({ op: "updateTrack", track: t.id, update: { pan: Number((e.target as HTMLInputElement).value) } }, false)}
              onPointerUp={(e) => change({ op: "updateTrack", track: t.id, update: { pan: Number((e.target as HTMLInputElement).value) } }, true)}
              onDoubleClick={() => void edit({ op: "updateTrack", track: t.id, update: { pan: 0 } })}
            />
          </div>,
          <ChainList track={t.id} effects={t.effects} selected={selected} onSelect={onSelect} />,
        ),
      )}
      {strip(
        "master",
        "Master",
        "#e6edf3",
        status?.master,
        p.master.gainDb,
        (db, final) => change({ op: "setMasterGain", gainDb: db }, final),
        null,
        <ChainList track={null} effects={p.master.effects} selected={selected} onSelect={onSelect} />,
      )}
      <div className="grow" />
    </div>
  );
}

/** EQ response curve computed from the same RBJ formulas as the DSP. */
function eqResponse(params: Record<string, number>, sr: number, f: number): number {
  const band = (shape: string, f0: number, q: number, g: number) => {
    const w0 = (2 * Math.PI * Math.min(f0, sr * 0.49)) / sr;
    const cos = Math.cos(w0);
    const sin = Math.sin(w0);
    const alpha = sin / (2 * q);
    const A = Math.pow(10, g / 40);
    let b0, b1, b2, a0, a1, a2;
    if (shape === "hp") {
      b0 = (1 + cos) / 2; b1 = -(1 + cos); b2 = (1 + cos) / 2; a0 = 1 + alpha; a1 = -2 * cos; a2 = 1 - alpha;
    } else if (shape === "peak") {
      b0 = 1 + alpha * A; b1 = -2 * cos; b2 = 1 - alpha * A; a0 = 1 + alpha / A; a1 = -2 * cos; a2 = 1 - alpha / A;
    } else {
      const sq = 2 * Math.sqrt(A) * alpha;
      if (shape === "ls") {
        b0 = A * (A + 1 - (A - 1) * cos + sq); b1 = 2 * A * (A - 1 - (A + 1) * cos); b2 = A * (A + 1 - (A - 1) * cos - sq);
        a0 = A + 1 + (A - 1) * cos + sq; a1 = -2 * (A - 1 + (A + 1) * cos); a2 = A + 1 + (A - 1) * cos - sq;
      } else {
        b0 = A * (A + 1 + (A - 1) * cos + sq); b1 = -2 * A * (A - 1 + (A + 1) * cos); b2 = A * (A + 1 + (A - 1) * cos - sq);
        a0 = A + 1 - (A - 1) * cos + sq; a1 = 2 * (A - 1 - (A + 1) * cos); a2 = A + 1 - (A - 1) * cos - sq;
      }
    }
    const w = (2 * Math.PI * f) / sr;
    const nr = b0 / a0 + (b1 / a0) * Math.cos(w) + (b2 / a0) * Math.cos(2 * w);
    const ni = -((b1 / a0) * Math.sin(w) + (b2 / a0) * Math.sin(2 * w));
    const dr = 1 + (a1 / a0) * Math.cos(w) + (a2 / a0) * Math.cos(2 * w);
    const di = -((a1 / a0) * Math.sin(w) + (a2 / a0) * Math.sin(2 * w));
    return 10 * Math.log10((nr * nr + ni * ni) / (dr * dr + di * di));
  };
  let db = params.output ?? 0;
  if ((params.hpf_on ?? 1) >= 0.5) db += band("hp", params.hpf_freq ?? 80, 0.707, 0);
  db += band("ls", params.low_freq ?? 120, 0.707, params.low_gain ?? 0);
  db += band("peak", params.mid1_freq ?? 400, params.mid1_q ?? 1, params.mid1_gain ?? 0);
  db += band("peak", params.mid2_freq ?? 3000, params.mid2_q ?? 1, params.mid2_gain ?? 0);
  db += band("hs", params.high_freq ?? 10000, 0.707, params.high_gain ?? 0);
  return db;
}

function EqCurve({ params, sr }: { params: Record<string, number>; sr: number }) {
  const w = 420;
  const h = 110;
  const pts: string[] = [];
  for (let i = 0; i <= 200; i++) {
    const f = 20 * Math.pow(1000, i / 200);
    const db = Math.max(-24, Math.min(24, eqResponse(params, sr, f)));
    pts.push(`${(i / 200) * w},${h / 2 - (db / 24) * (h / 2)}`);
  }
  return (
    <svg width={w} height={h} style={{ background: "var(--bg-sunken)", borderRadius: 6 }} role="img" aria-label="EQ response curve">
      {[100, 1000, 10000].map((f) => (
        <g key={f}>
          <line x1={(Math.log10(f / 20) / 3) * w} x2={(Math.log10(f / 20) / 3) * w} y1={0} y2={h} stroke="#222832" />
          <text x={(Math.log10(f / 20) / 3) * w + 3} y={h - 4} fontSize={9} fill="#6f7b88">{f >= 1000 ? `${f / 1000}k` : f}</text>
        </g>
      ))}
      <line x1={0} x2={w} y1={h / 2} y2={h / 2} stroke="#333b47" />
      <polyline points={pts.join(" ")} fill="none" stroke="#5cb0ff" strokeWidth={2} />
    </svg>
  );
}

export function EffectEditor({ state, slot }: { state: StudioState; slot: SlotRef }) {
  const { edit, change } = useStudioActions();
  const catalog = useCatalog();
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const [presets, setPresets] = useState<PresetEntry[]>([]);
  const [naming, setNaming] = useState<string | null>(null);
  const chain = slot.track ? state.project.tracks.find((t) => t.id === slot.track)?.effects : state.project.master.effects;
  const e = chain?.find((x) => x.id === slot.slot);
  const kind = e?.effect.type === "builtin" ? e.effect.kind : null;
  useEffect(() => {
    if (kind) api.presets(kind).then(setPresets).catch(error);
  }, [kind, error]);
  if (!e) return <div className="muted">Select an effect in the mixer.</div>;
  if (e.effect.type === "plugin") {
    return <Notice kind="info">Plugin “{e.effect.plugin.name}” — open the Plugins workspace to edit hosted plugin parameters.</Notice>;
  }
  const info = catalog.find((c) => c.id === kind);
  if (!info) return null;
  const params = e.effect.params;
  const trackName = slot.track ? state.project.tracks.find((t) => t.id === slot.track)?.name : "Master";
  return (
    <div className="stack">
      <div className="row row--between">
        <div className="row">
          <strong>{info.name}</strong>
          <span className="faint">on {trackName}</span>
          <Button size="sm" aria-pressed={!e.bypass} icon={<Power size={12} />} onClick={() => void edit({ op: "setEffectBypass", track: slot.track, slot: slot.slot, bypass: !e.bypass })}>
            {e.bypass ? "Bypassed" : "Active"}
          </Button>
        </div>
        <div className="row">
          <select
            className="select"
            aria-label="Load preset"
            value=""
            style={{ height: 26 }}
            onChange={(ev) => {
              const pr = presets.find((x) => x.id === ev.target.value);
              if (pr) void edit({ op: "setEffectParams", track: slot.track, slot: slot.slot, params: pr.params });
            }}
          >
            <option value="">Presets…</option>
            {presets.map((pr) => (
              <option key={pr.id} value={pr.id}>
                {pr.name}
              </option>
            ))}
          </select>
          <Button size="sm" icon={<Save size={12} />} onClick={() => setNaming("")}>
            Save preset
          </Button>
          <Button size="sm" onClick={() => void edit({ op: "setEffectParams", track: slot.track, slot: slot.slot, params: Object.fromEntries(info.params.map((p) => [p.id, p.default])) })}>
            Reset
          </Button>
        </div>
      </div>
      {kind === "eq" && <EqCurve params={params} sr={state.project.sampleRate} />}
      <div className="row row--wrap" style={{ gap: 10 }}>
        {info.params.map((p) => (
          <Knob
            key={p.id}
            param={p}
            value={params[p.id] ?? p.default}
            onChange={(v, final) => change({ op: "setEffectParam", track: slot.track, slot: slot.slot, param: p.id, value: v }, final)}
          />
        ))}
      </div>
      {naming != null && (
        <Modal
          title="Save preset"
          onClose={() => setNaming(null)}
          actions={
            <>
              <Button onClick={() => setNaming(null)}>Cancel</Button>
              <Button
                variant="primary"
                disabled={!naming.trim()}
                onClick={() => {
                  api
                    .savePreset(kind!, naming, params)
                    .then(() => {
                      toast("Preset saved", "ok");
                      setNaming(null);
                      return api.presets(kind!);
                    })
                    .then(setPresets)
                    .catch(error);
                }}
              >
                Save
              </Button>
            </>
          }
        >
          <input className="input" autoFocus style={{ width: "100%" }} placeholder="e.g. Bright vocal" value={naming} onChange={(ev) => setNaming(ev.target.value)} />
        </Modal>
      )}
    </div>
  );
}
