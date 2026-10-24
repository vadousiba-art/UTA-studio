import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, RefreshCw, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../api/client";
import type { PluginBundle, PluginCatalog } from "../api/types";
import { useJob } from "../components/controls";
import { Button, Empty, Notice, Panel, ProgressBar, Spinner, Switch, Tag } from "../components/ui";
import { useApp } from "../state/app";

let catalogListeners: Array<() => void> = [];
/** Lets other screens (mixer) refresh their plugin list after a scan. */
export function onPluginCatalogChanged(f: () => void) {
  catalogListeners.push(f);
  return () => {
    catalogListeners = catalogListeners.filter((x) => x !== f);
  };
}
function catalogChanged() {
  catalogListeners.forEach((f) => f());
}

function bundleName(b: PluginBundle) {
  const file = b.path.split(/[\\/]/).pop() ?? b.path;
  return b.plugins[0]?.name ?? file.replace(/\.vst3$/i, "");
}

export function Plugins() {
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const settings = useApp((s) => s.settings);
  const saveSettings = useApp((s) => s.saveSettings);
  const [cat, setCat] = useState<PluginCatalog | null>(null);
  const [scanJob, setScanJob] = useState<number | null>(null);

  const load = useCallback(() => api.pluginCatalog().then(setCat).catch(error), [error]);
  useEffect(() => {
    void load();
  }, [load]);

  const job = useJob(scanJob, (j) => {
    setScanJob(null);
    void load();
    catalogChanged();
    if (j.state === "done") {
      const r = j.result as { bundles: number; scanned: number; unchanged: number; failed: number; effects: number };
      toast(
        r.bundles === 0
          ? "No VST3 plugins were found in the scanned folders."
          : `Found ${r.effects} effect${r.effects === 1 ? "" : "s"} in ${r.bundles} bundle${r.bundles === 1 ? "" : "s"}${r.failed ? ` · ${r.failed} could not be used` : ""}.`,
        r.failed ? "warn" : "ok",
      );
    } else if (j.state === "failed") {
      error(j.error);
    }
  });

  const scan = async (force: boolean) => {
    try {
      setScanJob(await api.pluginScan(force));
    } catch (e) {
      error(e);
    }
  };

  const addFolder = async () => {
    const dir = await open({ directory: true, title: "Choose a folder that contains VST3 plugins" });
    if (typeof dir !== "string" || !settings) return;
    if (settings.pluginFolders.includes(dir)) return;
    await saveSettings({ pluginFolders: [...settings.pluginFolders, dir] });
    void load();
  };

  const removeFolder = async (dir: string) => {
    if (!settings) return;
    await saveSettings({ pluginFolders: settings.pluginFolders.filter((f) => f !== dir) });
    void load();
  };

  const setEnabled = async (b: PluginBundle, enabled: boolean) => {
    try {
      await api.pluginSetEnabled(b.path, enabled);
      await load();
      catalogChanged();
    } catch (e) {
      error(e);
    }
  };

  const forget = async (b: PluginBundle) => {
    try {
      await api.pluginForget(b.path);
      await load();
      catalogChanged();
    } catch (e) {
      error(e);
    }
  };

  const scanning = scanJob != null;
  const usable = cat?.bundles.filter((b) => !b.error && !b.missing) ?? [];
  const problems = cat?.bundles.filter((b) => b.error || b.missing) ?? [];

  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Plugin Manager</h1>
          <p className="muted">Third-party VST3 effects for the Recording Studio. Each plugin runs in its own helper process, so a crashing plugin cannot take your project down.</p>
        </div>
        <div className="row" style={{ gap: 8 }}>
          <Button icon={<RefreshCw size={14} />} variant="primary" disabled={scanning || !cat?.available} onClick={() => void scan(false)}>
            Scan for plugins
          </Button>
          <Button disabled={scanning || !cat?.available} onClick={() => void scan(true)} title="Scan every bundle again, even unchanged ones">
            Full rescan
          </Button>
        </div>
      </div>

      {!cat && <Spinner label="Loading plugin list…" />}
      {cat && !cat.available && (
        <Notice kind="warn">
          Plugin hosting is unavailable: the helper program <span className="mono">uta-plugin-host</span> was not found next to the app. Reinstall UTA Vocal Lab, or for development builds run <span className="mono">npm run sidecar</span>. Built-in effects keep working.
        </Notice>
      )}
      {scanning && (
        <Panel title="Scanning">
          <ProgressBar value={job?.progress ?? 0} label={job?.label ?? "Scanning plugins"} />
          <p className="muted" style={{ marginTop: 6 }}>Each plugin is loaded in a separate process. A plugin that crashes or hangs during the scan is marked as failed and skipped.</p>
          {scanJob != null && (
            <Button size="sm" onClick={() => void api.cancelJob(scanJob)}>
              Cancel
            </Button>
          )}
        </Panel>
      )}

      {cat && (
        <div className="grid grid--2" style={{ alignItems: "start" }}>
          <Panel title={`Installed effects (${usable.reduce((n, b) => n + b.plugins.filter((p) => !p.isInstrument).length, 0)})`}>
            {usable.length === 0 ? (
              <Empty title="No plugins yet" action={cat.available ? <Button onClick={() => void scan(false)}>Scan for plugins</Button> : undefined}>
                Install VST3 effects with their own installers (or copy the <span className="mono">.vst3</span> bundle into a plugin folder), then scan.
              </Empty>
            ) : (
              <div className="col" style={{ gap: 6 }}>
                {usable.map((b) => (
                  <div key={b.path} className="row" style={{ gap: 10, padding: 8, border: "1px solid var(--border)", borderRadius: 6, opacity: b.enabled ? 1 : 0.6 }}>
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div style={{ fontWeight: 600 }}>
                        {bundleName(b)} {b.plugins.some((p) => p.isInstrument) && <Tag kind="warn">instrument — not supported</Tag>}
                      </div>
                      <div className="muted" style={{ fontSize: 12 }}>
                        {b.plugins.map((p) => `${p.name}${p.version ? ` ${p.version}` : ""}`).join(", ")} · {b.plugins[0]?.vendor || "unknown vendor"}
                      </div>
                      <div className="mono muted" style={{ fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={b.path}>
                        {b.path}
                      </div>
                    </div>
                    <Switch checked={b.enabled} label={b.enabled ? "Enabled" : "Disabled"} onChange={(v) => void setEnabled(b, v)} />
                  </div>
                ))}
              </div>
            )}
          </Panel>

          <div className="col" style={{ gap: 12 }}>
            {problems.length > 0 && (
              <Panel title={`Could not be used (${problems.length})`}>
                <div className="col" style={{ gap: 6 }}>
                  {problems.map((b) => (
                    <div key={b.path} className="row" style={{ gap: 8, alignItems: "flex-start" }}>
                      <div className="grow" style={{ minWidth: 0 }}>
                        <div style={{ fontWeight: 600 }}>{bundleName(b)}</div>
                        <div style={{ fontSize: 12, color: "var(--warn)" }}>{b.missing ? "No longer installed." : b.error}</div>
                        <div className="mono muted" style={{ fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={b.path}>
                          {b.path}
                        </div>
                      </div>
                      <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} aria-label={`Remove ${bundleName(b)} from the list`} title="Remove from the list" onClick={() => void forget(b)} />
                    </div>
                  ))}
                </div>
              </Panel>
            )}

            <Panel title="Plugin folders" actions={<Button size="sm" icon={<FolderPlus size={13} />} onClick={() => void addFolder()}>Add folder</Button>}>
              <div className="col" style={{ gap: 4, fontSize: 12 }}>
                {cat.defaultFolders.map((f) => (
                  <div key={f} className="row" style={{ gap: 6 }}>
                    <span className="mono grow">{f}</span>
                    <Tag>standard</Tag>
                  </div>
                ))}
                {cat.userFolders.map((f) => (
                  <div key={f} className="row" style={{ gap: 6 }}>
                    <span className="mono grow">{f}</span>
                    <Button size="sm" variant="ghost" icon={<X size={12} />} aria-label={`Stop scanning ${f}`} onClick={() => void removeFolder(f)} />
                  </div>
                ))}
              </div>
            </Panel>

            <Panel title="How plugins work here">
              <ul className="muted" style={{ margin: 0, paddingLeft: 18, fontSize: 13, lineHeight: 1.5 }}>
                <li>Only 64-bit VST3 audio effects are supported. Instruments, VST2, CLAP and 32-bit plugins are not.</li>
                <li>Add a plugin from the <b>+ Add effect</b> list in the Studio mixer. Its parameters appear as sliders in the effect panel; plugin editor windows are not shown in this version.</li>
                <li>Plugins add about 21 ms of buffering (more if the plugin itself has latency). Playback stays in sync, but monitoring through plugins while recording is delayed — record dry and add plugins afterwards.</li>
                <li>If a plugin crashes, the slot passes the unprocessed signal and shows <b>Reload</b>. Projects keep plugin settings even when the plugin is missing on this computer.</li>
                <li>UTA Vocal Lab never downloads or installs plugins. Only install plugins from sources you trust.</li>
              </ul>
            </Panel>
          </div>
        </div>
      )}
    </div>
  );
}
