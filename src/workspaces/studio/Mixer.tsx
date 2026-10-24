import { AlertTriangle, ArrowDown, ArrowUp, Power, Save, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../api/client";
import type { BuiltinKind, EffectCatalogEntry, EffectSlot, MeterValue, PluginParams, PluginRef, PresetEntry, SlotStatus, StudioState } from "../../api/types";
import { onPluginCatalogChanged } from "../Plugins";
import { Fader, Knob } from "../../components/controls";
import { Button, Modal, Notice, Spinner, Tag } from "../../components/ui";
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

/** Enabled, scanned VST3 effects that can be inserted. */
export function useInstalledPlugins() {
  const [list, setList] = useState<PluginRef[]>([]);
  useEffect(() => {
    const load = () =>
      api
        .pluginCatalog()
        .then((c) =>
          setList(
            c.available
              ? c.bundles
                  .filter((b) => b.enabled && !b.error && !b.missing)
                  .flatMap((b) =>
                    b.plugins
                      .filter((p) => !p.isInstrument)
                      .map((p) => ({ format: "vst3", uid: p.uid, name: p.name, vendor: p.vendor, version: p.version, path: b.path })),
                  )
              : [],
          ),
        )
        .catch(() => setList([]));
    void load();
    return onPluginCatalogChanged(load);
  }, []);
  return list;
}

function ChainList({ track, effects, selected, onSelect, slots }: { track: string | null; effects: EffectSlot[]; selected: SlotRef | null; onSelect: (s: SlotRef | null) => void; slots: SlotStatus[] }) {
  const { edit } = useStudioActions();
  const catalog = useCatalog();
  const plugins = useInstalledPlugins();
  return (
    <div className="col" style={{ gap: 3, width: "100%" }}>
      {effects.map((e, i) => {
        const name = e.effect.type === "builtin" ? catalog.find((c) => c.id === (e.effect as { kind: string }).kind)?.name ?? e.effect.kind : e.effect.plugin.name;
        const sel = selected?.slot === e.id;
        const ps = e.effect.type === "plugin" ? slots.find((x) => x.slot === e.id) : undefined;
        const problem = ps && ps.status !== "running" ? ps : undefined;
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
            <span className="grow" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", opacity: e.bypass ? 0.5 : 1 }} title={problem?.message ?? name}>
              {name}
            </span>
            {problem && (
              <span role="img" aria-label={`${name}: ${problem.status}`} title={problem.message ?? problem.status} style={{ color: problem.status === "crashed" ? "var(--danger)" : "var(--warn)", display: "flex" }}>
                <AlertTriangle size={11} />
              </span>
            )}
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
          const plugin = v.startsWith("vst3:") ? plugins[Number(v.slice(5))] : undefined;
          const st = plugin
            ? await edit({ op: "insertPlugin", track, plugin, index: null })
            : await edit({ op: "insertEffect", track, effect: v as BuiltinKind, index: null });
          const chain = track ? st?.project.tracks.find((t) => t.id === track)?.effects : st?.project.master.effects;
          const last = chain?.[chain.length - 1];
          if (last) onSelect({ track, slot: last.id });
        }}
      >
        <option value="">+ Add effect…</option>
        <optgroup label="Built-in">
          {catalog.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </optgroup>
        {plugins.length > 0 && (
          <optgroup label="VST3 plugins">
            {plugins.map((p, i) => (
              <option key={`${p.uid}${p.path}`} value={`vst3:${i}`}>
                {p.name}
              </option>
            ))}
          </optgroup>
        )}
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
          <ChainList track={t.id} effects={t.effects} selected={selected} onSelect={onSelect} slots={state.pluginSlots} />,
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
        <ChainList track={null} effects={p.master.effects} selected={selected} onSelect={onSelect} slots={state.pluginSlots} />,
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
    return <PluginEditor state={state} slot={slot} effect={e} />;
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

/** Generic controls for a hosted VST3 plugin (plugin editor windows are not shown). */
function PluginEditor({ state, slot, effect }: { state: StudioState; slot: SlotRef; effect: EffectSlot }) {
  const { edit, change, apply } = useStudioActions();
  const installed = useInstalledPlugins();
  const [params, setParams] = useState<PluginParams | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const plugin = effect.effect.type === "plugin" ? effect.effect.plugin : null;
  const stored = effect.effect.type === "plugin" ? effect.effect.params : {};
  const status = state.pluginSlots.find((s) => s.slot === slot.slot);
  const running = status?.status === "running";
  const trackName = slot.track ? state.project.tracks.find((t) => t.id === slot.track)?.name : "Master";
  const storedKey = JSON.stringify(stored);

  const refresh = useCallback(() => {
    if (!running) {
      setParams(null);
      return;
    }
    api
      .studioPluginParams(slot.track, slot.slot)
      .then((p) => {
        setParams(p);
        setLoadError(null);
      })
      .catch((e) => setLoadError(String((e as { message?: string }).message ?? e)));
  }, [running, slot.track, slot.slot]);

  // reload values when the slot, its status or its stored values change (undo/redo)
  useEffect(() => {
    refresh();
  }, [refresh, storedKey]);

  if (!plugin) return null;
  const sr = state.project.sampleRate;
  const visible = (params?.params ?? []).filter((p) => !p.isBypass && (!filter || p.title.toLowerCase().includes(filter.toLowerCase())));
  const valueOf = (id: number) => params?.values.find((v) => v.id === id);
  const alternatives = installed.filter((p) => p.uid === plugin.uid || p.name === plugin.name);

  return (
    <div className="stack">
      <div className="row row--between">
        <div className="row">
          <strong>{plugin.name}</strong>
          <span className="faint">
            {plugin.vendor} · on {trackName}
          </span>
          <Button size="sm" aria-pressed={!effect.bypass} icon={<Power size={12} />} onClick={() => void edit({ op: "setEffectBypass", track: slot.track, slot: slot.slot, bypass: !effect.bypass })}>
            {effect.bypass ? "Bypassed" : "Active"}
          </Button>
          {status && <Tag kind={running ? "ok" : status.status === "crashed" ? "danger" : "warn"}>{status.status}</Tag>}
          {running && status && <span className="faint">adds {((status.latencyFrames / sr) * 1000).toFixed(0)} ms (compensated)</span>}
        </div>
        <div className="row">
          {params && params.params.length > 12 && (
            <input className="input" style={{ height: 26, width: 160 }} placeholder="Find parameter…" aria-label="Find parameter" value={filter} onChange={(e) => setFilter(e.target.value)} />
          )}
          {(status?.status === "crashed" || status?.status === "failed") && (
            <Button size="sm" variant="primary" onClick={() => void apply(api.studioReloadPlugin(slot.track, slot.slot))}>
              Reload plugin
            </Button>
          )}
        </div>
      </div>
      {status && !running && <Notice kind={status.status === "crashed" ? "danger" : "warn"}>{status.message}</Notice>}
      {status?.status === "missing" && (
        <div className="row" style={{ gap: 8 }}>
          {alternatives.length > 0 ? (
            <>
              <span className="muted">Use the installed copy:</span>
              {alternatives.map((a) => (
                <Button key={a.path} size="sm" onClick={() => void edit({ op: "relinkPlugin", track: slot.track, slot: slot.slot, plugin: a })} title={a.path}>
                  {a.name} {a.version}
                </Button>
              ))}
            </>
          ) : (
            <span className="muted">Install the plugin on this computer and scan for it in the Plugin Manager; this slot will then load with its saved settings.</span>
          )}
        </div>
      )}
      {status?.warnings.map((w) => (
        <Notice key={w} kind="warn">
          {w}
        </Notice>
      ))}
      {running && loadError && <Notice kind="danger">{loadError}</Notice>}
      {running && !params && !loadError && <Spinner label="Reading parameters…" />}
      {running && params && visible.length === 0 && <div className="muted">This plugin exposes no adjustable parameters.</div>}
      {running && params && (
        <div className="grid grid--3" style={{ gap: "6px 16px", maxHeight: 200, overflowY: "auto", paddingRight: 4 }}>
          {visible.map((p) => {
            const v = valueOf(p.id);
            const step = p.stepCount > 0 ? 1 / p.stepCount : 0.001;
            return (
              <label key={p.id} className="col" style={{ gap: 2, fontSize: 12 }}>
                <span className="row row--between">
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.title}</span>
                  <span className="mono faint">
                    {v?.display ?? ""} {p.units}
                  </span>
                </span>
                <input
                  className="slider"
                  type="range"
                  min={0}
                  max={1}
                  step={step}
                  disabled={p.readOnly}
                  aria-label={p.title}
                  defaultValue={v?.value ?? p.defaultNormalized}
                  key={`${p.id}:${v?.value ?? ""}`}
                  onInput={(e) => change({ op: "setPluginParam", track: slot.track, slot: slot.slot, id: p.id, value: Number((e.target as HTMLInputElement).value) }, false)}
                  onPointerUp={(e) => {
                    void edit({ op: "setPluginParam", track: slot.track, slot: slot.slot, id: p.id, value: Number((e.target as HTMLInputElement).value) }).then(() => refresh());
                  }}
                  onKeyUp={(e) => {
                    void edit({ op: "setPluginParam", track: slot.track, slot: slot.slot, id: p.id, value: Number((e.target as HTMLInputElement).value) }).then(() => refresh());
                  }}
                  onDoubleClick={() => void edit({ op: "setPluginParam", track: slot.track, slot: slot.slot, id: p.id, value: p.defaultNormalized }).then(() => refresh())}
                />
              </label>
            );
          })}
        </div>
      )}
      <div className="faint" style={{ fontSize: 11 }}>
        Plugin windows are not available in this version; every automatable parameter is shown here. Double-click a slider to reset it.
      </div>
    </div>
  );
}
