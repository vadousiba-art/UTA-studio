import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/client";
import type { DeviceList } from "../api/types";
import { Button, Confirm, Field, Notice, Panel, Switch, Tag } from "../components/ui";
import { noteName } from "../lib/music";
import { useApp } from "../state/app";
import { Meter } from "../viz/Meter";

export function SettingsView() {
  const settings = useApp((s) => s.settings);
  const info = useApp((s) => s.info);
  const status = useApp((s) => s.status);
  const saveSettings = useApp((s) => s.saveSettings);
  const toast = useApp((s) => s.toast);
  const error = useApp((s) => s.error);
  const refreshInfo = useApp((s) => s.refreshInfo);
  const [devices, setDevices] = useState<DeviceList | null>(null);
  const [devError, setDevError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const loadDevices = () => {
    setDevError(null);
    api.listDevices().then(setDevices).catch((e) => setDevError(e.message));
  };
  useEffect(loadDevices, []);

  if (!settings || !info) return null;
  const output = devices?.outputs.find((d) => d.id === settings.outputDevice) ?? devices?.outputs.find((d) => d.isDefault);
  const rates = output?.sampleRates ?? [44100, 48000];
  const engine = status?.backend ?? info.engine;
  const reveal = (p: string) => revealItemInDir(p).catch(error);

  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Settings</h1>
          <p>Audio devices, privacy, practice preferences and your data.</p>
        </div>
      </div>
      <div className="grid grid--2">
        <Panel
          title="Audio devices"
          actions={
            <Button size="sm" icon={<RefreshCw size={14} />} onClick={() => { loadDevices(); api.restartEngine().then(() => refreshInfo()).catch(error); }}>
              Rescan & restart
            </Button>
          }
        >
          <div className="stack">
            {devError && <Notice kind="danger">{devError}</Notice>}
            {info.engineError && <Notice kind="warn">{info.engineError}</Notice>}
            <Field label="Output (speakers / headphones)">
              <select className="select" value={settings.outputDevice ?? ""} onChange={(e) => void saveSettings({ outputDevice: e.target.value || null })}>
                <option value="">System default</option>
                {devices?.outputs.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                    {d.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Input (microphone / interface)" hint="Use headphones while recording so the backing track isn't picked up by the mic.">
              <select className="select" value={settings.inputDevice ?? ""} disabled={!settings.microphoneEnabled} onChange={(e) => void saveSettings({ inputDevice: e.target.value || null })}>
                <option value="">System default</option>
                {devices?.inputs.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name} ({d.maxChannels} ch){d.isDefault ? " (default)" : ""}
                  </option>
                ))}
              </select>
            </Field>
            <div className="grid grid--2">
              <Field label="Sample rate">
                <select className="select" value={settings.sampleRate ?? ""} onChange={(e) => void saveSettings({ sampleRate: e.target.value ? Number(e.target.value) : null })}>
                  <option value="">Device default</option>
                  {rates.map((r) => (
                    <option key={r} value={r}>
                      {r / 1000} kHz
                    </option>
                  ))}
                </select>
              </Field>
              <Field label="Buffer size" hint="Smaller = lower latency, more CPU. Increase if you hear clicks.">
                <select className="select" value={settings.bufferFrames ?? ""} onChange={(e) => void saveSettings({ bufferFrames: e.target.value ? Number(e.target.value) : null })}>
                  <option value="">Driver default</option>
                  {[128, 256, 512, 1024, 2048].map((b) => (
                    <option key={b} value={b}>
                      {b} frames
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            <Field label="Input channel for Analyzer and Coach">
              <select className="select" value={settings.inputChannel} onChange={(e) => void saveSettings({ inputChannel: Number(e.target.value) })}>
                {Array.from({ length: Math.max(1, engine?.inputChannels ?? 1) }, (_, i) => (
                  <option key={i} value={i}>
                    Input {i + 1}
                  </option>
                ))}
              </select>
            </Field>
            {engine && (
              <div className="panel panel--flat">
                <div className="row row--between">
                  <span className="muted">Running</span>
                  <Tag kind={engine.kind === "device" ? "ok" : "warn"}>{engine.kind === "device" ? `${engine.host}` : "silent fallback"}</Tag>
                </div>
                <div className="mono faint" style={{ fontSize: 11, marginTop: 6 }}>
                  {engine.outputName ?? "no output"} · {engine.sampleRate} Hz · {engine.bufferFrames ?? "?"} frames
                  <br />
                  input: {engine.inputName ?? "none"} ({engine.inputChannels} ch)
                </div>
                {status && status.inputs.length > 0 && (
                  <div className="row" style={{ marginTop: 10 }}>
                    <span className="faint" style={{ fontSize: 11 }}>Input level</span>
                    <Meter values={status.inputs.slice(0, 2)} orientation="horizontal" length={220} onResetClip={() => void api.resetClips()} label="Input" />
                  </div>
                )}
              </div>
            )}
          </div>
        </Panel>

        <Panel title="Privacy">
          <div className="stack">
            <Switch
              checked={settings.microphoneEnabled}
              onChange={(v) => void saveSettings({ microphoneEnabled: v })}
              label={<span>Allow microphone access {settings.microphoneEnabled ? <Tag kind="ok">on</Tag> : <Tag>off</Tag>}</span>}
            />
            <div className="muted">
              All processing happens on this computer. UTA Vocal Lab has no account, sends nothing over the network, and never uses
              your recordings for training.
            </div>
            <Field label="Your data">
              <div className="col">
                {[
                  ["App data & practice recordings", info.paths.dataDir],
                  ["Projects", info.paths.projectsDir],
                  ["Backups", info.paths.backupsDir],
                ].map(([label, p]) => (
                  <div key={p} className="row row--between">
                    <div>
                      <div>{label}</div>
                      <div className="mono faint" style={{ fontSize: 11 }}>{p}</div>
                    </div>
                    <Button size="sm" icon={<FolderOpen size={14} />} onClick={() => void reveal(p!)}>
                      Show
                    </Button>
                  </div>
                ))}
              </div>
            </Field>
            <div className="row">
              <Button onClick={() => api.backupLibrary().then((p) => toast(`Library backed up to ${p}`, "ok")).catch(error)}>Back up practice library</Button>
              <Button variant="danger" icon={<Trash2 size={14} />} onClick={() => setConfirmDelete(true)}>
                Delete training history
              </Button>
            </div>
          </div>
        </Panel>

        <Panel title="Practice">
          <div className="stack">
            <Field label="Comfortable range" hint="Measure it in Coach → Range check. Exercises never go outside it.">
              <div className="row">
                <span className="mono">
                  {noteName(settings.voiceRange.low)} – {noteName(settings.voiceRange.high)}
                </span>
                {!settings.voiceRangeMeasured && <Tag kind="warn">default, not measured</Tag>}
              </div>
            </Field>
            <Field label="Default session length">
              <select className="select" value={settings.sessionLength} onChange={(e) => void saveSettings({ sessionLength: e.target.value as "short" | "long" })}>
                <option value="short">Short (~10 minutes)</option>
                <option value="long">Long (~25 minutes)</option>
              </select>
            </Field>
            <Field label={`Reference tone volume (${Math.round(settings.referenceVolume * 100)}%)`}>
              <input className="slider" type="range" min={0} max={1} step={0.05} value={settings.referenceVolume} onChange={(e) => void saveSettings({ referenceVolume: Number(e.target.value) })} />
            </Field>
            <Switch checked={settings.showKana} onChange={(v) => void saveSettings({ showKana: v })} label="Show kana in Song Lab" />
            <Switch checked={settings.showRomaji} onChange={(v) => void saveSettings({ showRomaji: v })} label="Show romaji in Song Lab" />
          </div>
        </Panel>

        <Panel title="Performance & recording">
          <div className="stack">
            <Switch
              checked={settings.lightweightMode}
              onChange={(v) => void saveSettings({ lightweightMode: v })}
              label="Lightweight mode (coarser analysis, less CPU — for slower computers)"
            />
            <Field label="Analysis resolution" hint="Time between pitch measurements. 10 ms is detailed; 20 ms uses about half the CPU.">
              <select className="select" value={settings.analysisHopMs} disabled={settings.lightweightMode} onChange={(e) => void saveSettings({ analysisHopMs: Number(e.target.value) })}>
                <option value={5}>5 ms (very detailed)</option>
                <option value={10}>10 ms (default)</option>
                <option value={20}>20 ms (light)</option>
              </select>
            </Field>
            <Field label={`Recording latency correction (${settings.latencyOffsetMs > 0 ? "+" : ""}${settings.latencyOffsetMs} ms)`} hint="If recorded takes sound early or late against the backing, adjust this. Positive moves takes earlier.">
              <input className="slider" type="range" min={-200} max={200} step={1} value={settings.latencyOffsetMs} onChange={(e) => void saveSettings({ latencyOffsetMs: Number(e.target.value) })} />
            </Field>
            <Field label="Autosave interval">
              <select className="select" value={settings.autosaveMinutes} onChange={(e) => void saveSettings({ autosaveMinutes: Number(e.target.value) })}>
                {[1, 2, 5, 10].map((m) => (
                  <option key={m} value={m}>
                    every {m} minute{m === 1 ? "" : "s"}
                  </option>
                ))}
              </select>
            </Field>
            <div className="faint" style={{ fontSize: 11 }}>
              UTA Vocal Lab {info.version} · {info.platform}
            </div>
          </div>
        </Panel>
      </div>
      {confirmDelete && (
        <Confirm
          title="Delete training history?"
          danger
          confirmLabel="Delete history"
          onCancel={() => setConfirmDelete(false)}
          onConfirm={() => {
            setConfirmDelete(false);
            api.deleteTrainingHistory().then(() => toast("Training history deleted", "ok")).catch(error);
          }}
        >
          This removes all lesson attempts, skill levels, review schedules and notes. Recordings and projects are not deleted.
          Consider backing up the practice library first.
        </Confirm>
      )}
    </div>
  );
}
