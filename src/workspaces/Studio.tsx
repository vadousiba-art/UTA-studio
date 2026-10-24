import { open } from "@tauri-apps/plugin-dialog";
import {
  Circle,
  FileAudio,
  FolderClosed,
  History,
  Import,
  Metronome as MetronomeIcon,
  Plus,
  Redo2,
  Repeat,
  Save,
  SkipBack,
  Square,
  Play,
  Undo2,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../api/client";
import type { RevisionInfo } from "../api/types";
import { useJob } from "../components/controls";
import { Button, Confirm, Modal, Notice, ProgressBar, Tabs } from "../components/ui";
import { formatTime } from "../lib/music";
import { useApp } from "../state/app";
import { ExportDialog } from "./studio/ExportDialog";
import { Inspector } from "./studio/Inspector";
import { EffectEditor, Mixer, type SlotRef } from "./studio/Mixer";
import { StudioStart } from "./studio/StudioStart";
import { Timeline } from "./studio/Timeline";
import { useStudioActions } from "./studio/useStudio";

const isTyping = (t: EventTarget | null) => t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement;

export function Studio() {
  const state = useApp((s) => s.studio);
  const status = useApp((s) => s.status);
  const settings = useApp((s) => s.settings);
  const setStudio = useApp((s) => s.setStudio);
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const { edit, undo, redo, save, apply, refresh } = useStudioActions();
  const [pps, setPps] = useState(60);
  const [snap, setSnap] = useState<"off" | "beat" | "bar">("beat");
  const [clip, setClip] = useState<string | null>(null);
  const [track, setTrack] = useState<string | null>(null);
  const [selection, setSelection] = useState<[number, number] | null>(null);
  const [slot, setSlot] = useState<SlotRef | null>(null);
  const [bottom, setBottom] = useState<"mixer" | "inspector" | "effect" | "notes">("mixer");
  const [exporting, setExporting] = useState(false);
  const [importJob, setImportJob] = useState<number | null>(null);
  const [revisions, setRevisions] = useState<RevisionInfo[] | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [recStart, setRecStart] = useState<number | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const imp = useJob(importJob, (j) => {
    setImportJob(null);
    if (j.state === "done") {
      toast("Import finished", "ok");
      void refresh();
    } else if (j.state === "failed") toast(`Import failed: ${j.error}`, "error");
  });

  const sr = state?.project.sampleRate ?? 48000;
  const playhead = (status?.position ?? 0) / sr;
  const recording = state?.recording ?? false;
  const playing = status?.playing ?? false;

  const togglePlay = useCallback(async () => {
    try {
      if (recording) {
        const r = await api.studioStopRecording();
        setRecStart(null);
        r.warnings.forEach((w) => toast(w, "warn"));
        await refresh();
      } else if (playing) await api.studioStop();
      else await api.studioPlay(selection ? Math.round(selection[0] * sr) : undefined);
    } catch (e) {
      error(e);
    }
  }, [recording, playing, selection, sr, error, toast, refresh]);

  const toggleRecord = useCallback(async () => {
    try {
      if (recording) {
        const r = await api.studioStopRecording();
        setRecStart(null);
        r.warnings.forEach((w) => toast(w, "warn"));
        await refresh();
      } else {
        const from = Math.round(playhead * sr);
        await api.studioRecord(from);
        setRecStart(playhead);
        await refresh();
      }
    } catch (e) {
      error(e);
    }
  }, [recording, playhead, sr, error, toast, refresh]);

  const importFiles = async () => {
    const files = await open({ multiple: true, title: "Import WAV files", filters: [{ name: "WAV audio", extensions: ["wav", "wave"] }] });
    if (!files || (Array.isArray(files) && files.length === 0)) return;
    const list = Array.isArray(files) ? files : [files];
    try {
      setImportJob(await api.studioImport(list, track, Math.round(playhead * sr)));
    } catch (e) {
      error(e);
    }
  };

  useEffect(() => {
    if (!state) return;
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || document.querySelector(".modal")) return;
      const mod = e.ctrlKey || e.metaKey;
      if (e.code === "Space") {
        e.preventDefault();
        void togglePlay();
      } else if (!mod && e.key.toLowerCase() === "r") {
        void toggleRecord();
      } else if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save().then((s) => s && toast("Saved", "ok"));
      } else if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) {
        e.preventDefault();
        void undo();
      } else if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey))) {
        e.preventDefault();
        void redo();
      } else if ((e.key === "Delete" || e.key === "Backspace") && clip) {
        void edit({ op: "deleteClip", clip });
        setClip(null);
      } else if (!mod && e.key.toLowerCase() === "s" && clip) {
        void edit({ op: "splitClip", clip, at: Math.round(playhead * sr) });
      } else if (mod && e.key.toLowerCase() === "d" && clip) {
        e.preventDefault();
        void edit({ op: "duplicateClip", clip, position: null });
      } else if (e.key === "Home") {
        void api.studioLocate(0).catch(error);
      } else if (!mod && e.key.toLowerCase() === "l") {
        if (state.project.loopRegion) void edit({ op: "setLoop", start: null, end: null });
        else if (selection) void edit({ op: "setLoop", start: Math.round(selection[0] * sr), end: Math.round(selection[1] * sr) });
      } else if (!mod && e.key.toLowerCase() === "m") {
        void edit({ op: "setMetronome", metronome: { ...state.project.metronome, enabled: !state.project.metronome.enabled } });
      } else if (e.key === "+" || e.key === "=") setPps((p) => Math.min(800, p * 1.4));
      else if (e.key === "-") setPps((p) => Math.max(8, p / 1.4));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state, clip, selection, playhead, sr, togglePlay, toggleRecord, save, undo, redo, edit, error, toast]);

  if (!state) return <StudioStart />;
  const p = state.project;
  const missing = state.assetChecks;

  return (
    <div className="workspace workspace--flush">
      {/* toolbar */}
      <div className="row" style={{ padding: "8px 12px", borderBottom: "1px solid var(--border)", background: "var(--panel)", gap: 6, flexWrap: "wrap" }}>
        <strong style={{ fontSize: 15, marginRight: 4 }} title={state.path}>
          {p.name}
          {state.dirty && <span className="faint"> •</span>}
        </strong>
        <Button size="sm" icon={<Save size={14} />} title="Save (Ctrl+S)" onClick={() => void save().then((s) => s && toast("Saved", "ok"))}>Save</Button>
        <Button size="sm" icon={<Undo2 size={14} />} disabled={!state.canUndo} title={`Undo ${state.undoLabel ?? ""} (Ctrl+Z)`} aria-label="Undo" onClick={() => void undo()} />
        <Button size="sm" icon={<Redo2 size={14} />} disabled={!state.canRedo} title={`Redo ${state.redoLabel ?? ""} (Ctrl+Y)`} aria-label="Redo" onClick={() => void redo()} />
        <span style={{ width: 10 }} />
        <Button size="sm" icon={<SkipBack size={14} />} aria-label="Go to start" title="Go to start (Home)" onClick={() => api.studioLocate(0).catch(error)} />
        <Button size="sm" variant={playing && !recording ? "primary" : "default"} icon={playing ? <Square size={14} /> : <Play size={14} />} aria-label={playing ? "Stop" : "Play"} title="Play / stop (Space)" onClick={() => void togglePlay()} />
        <Button size="sm" variant="rec" icon={recording ? <Square size={14} /> : <Circle size={14} fill="currentColor" />} disabled={!settings?.microphoneEnabled} title="Record armed tracks (R)" onClick={() => void toggleRecord()}>
          {recording ? "Stop" : "Rec"}
        </Button>
        <span className="mono" style={{ minWidth: 86, textAlign: "center", fontSize: 14 }}>{formatTime(playhead, true)}</span>
        <Button
          size="sm"
          icon={<Repeat size={14} />}
          aria-pressed={!!p.loopRegion}
          title={p.loopRegion ? "Clear loop (L)" : "Loop the selection (L)"}
          disabled={!p.loopRegion && !selection}
          onClick={() => void (p.loopRegion ? edit({ op: "setLoop", start: null, end: null }) : selection && edit({ op: "setLoop", start: Math.round(selection[0] * sr), end: Math.round(selection[1] * sr) }))}
        />
        <Button size="sm" icon={<MetronomeIcon size={14} />} aria-pressed={p.metronome.enabled} title="Metronome (M)" onClick={() => void edit({ op: "setMetronome", metronome: { ...p.metronome, enabled: !p.metronome.enabled } })} />
        <input
          className="input mono"
          type="number"
          min={20}
          max={300}
          aria-label="Tempo (BPM)"
          title="Tempo (BPM)"
          defaultValue={p.tempoBpm}
          key={`bpm${p.tempoBpm}`}
          style={{ width: 64, height: 26 }}
          onBlur={(e) => Number(e.target.value) !== p.tempoBpm && void edit({ op: "setTempo", bpm: Number(e.target.value) })}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        />
        <span className="faint" style={{ fontSize: 11 }}>BPM</span>
        <select className="select" aria-label="Snap" value={snap} onChange={(e) => setSnap(e.target.value as typeof snap)} style={{ height: 26 }}>
          <option value="off">Snap off</option>
          <option value="beat">Snap: beat</option>
          <option value="bar">Snap: bar</option>
        </select>
        <Button size="sm" icon={<ZoomOut size={14} />} aria-label="Zoom out" onClick={() => setPps((x) => Math.max(8, x / 1.4))} />
        <Button size="sm" icon={<ZoomIn size={14} />} aria-label="Zoom in" onClick={() => setPps((x) => Math.min(800, x * 1.4))} />
        <span className="grow" />
        <Button size="sm" icon={<Plus size={14} />} onClick={() => void edit({ op: "addTrack", name: null })}>Track</Button>
        <Button size="sm" icon={<Import size={14} />} disabled={importJob != null} onClick={() => void importFiles()}>Import</Button>
        <Button size="sm" icon={<FileAudio size={14} />} onClick={() => setExporting(true)}>Export</Button>
        <Button size="sm" icon={<History size={14} />} aria-label="Saved versions" title="Saved versions and backups" onClick={() => api.studioRevisions().then(setRevisions).catch(error)} />
        <Button size="sm" icon={<FolderClosed size={14} />} aria-label="Close project" title="Close project" onClick={() => (state.dirty ? setConfirmClose(true) : void api.studioClose(false).then(() => setStudio(null)).catch(error))} />
      </div>

      {(state.warnings.length > 0 || missing.length > 0 || state.newerAutosave || imp) && (
        <div className="col" style={{ padding: "8px 12px", gap: 6 }}>
          {state.newerAutosave && (
            <Notice kind="info">
              Unsaved changes from {new Date(state.newerAutosave.createdAt).toLocaleString()} were kept by autosave.{" "}
              <Button size="sm" onClick={() => void apply(api.studioLoadRevision(state.newerAutosave!.id))}>Restore them</Button>
            </Notice>
          )}
          {state.warnings.map((w) => (
            <Notice key={w} kind="warn">{w}</Notice>
          ))}
          {missing.map((m) => (
            <Notice key={m.assetId} kind="warn">
              Audio file {m.file}: {m.status.status}
              {m.status.reason ? ` (${m.status.reason})` : ""}.{" "}
              <Button
                size="sm"
                onClick={async () => {
                  const f = await open({ title: "Choose the replacement WAV file", filters: [{ name: "WAV audio", extensions: ["wav"] }] });
                  if (typeof f === "string") void apply(api.studioRelinkAsset(m.assetId, f));
                }}
              >
                Relink…
              </Button>
            </Notice>
          ))}
          {imp && (
            <div className="row">
              <span className="muted">{imp.label}</span>
              <div className="grow"><ProgressBar value={imp.progress} /></div>
            </div>
          )}
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        {p.tracks.length === 0 ? (
          <div className="empty" style={{ flex: 1 }}>
            <h3>No tracks</h3>
            <Button icon={<Plus size={14} />} onClick={() => void edit({ op: "addTrack", name: "Vocal" })}>Add a track</Button>
          </div>
        ) : (
          <Timeline
            state={state}
            pps={pps}
            snap={snap}
            selectedClip={clip}
            onSelectClip={(c, t) => {
              setClip(c);
              if (t) setTrack(t);
              if (c) setBottom("inspector");
            }}
            selectedTrack={track}
            onSelectTrack={setTrack}
            selection={selection}
            onSelection={setSelection}
            playhead={playhead}
            recordingSince={recording ? recStart : null}
          />
        )}
      </div>

      <div style={{ height: 300, borderTop: "1px solid var(--border)", background: "var(--panel)", display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "0 12px" }}>
          <Tabs
            tabs={[
              { id: "mixer", label: "Mixer" },
              { id: "effect", label: "Effect" },
              { id: "inspector", label: "Clip / Takes" },
              { id: "notes", label: "Session notes" },
            ]}
            value={bottom}
            onChange={setBottom}
          />
        </div>
        <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: bottom === "mixer" ? 0 : "0 16px 12px" }}>
          {bottom === "mixer" && (
            <Mixer
              state={state}
              selected={slot}
              onSelect={(s) => {
                setSlot(s);
                if (s) setBottom("effect");
              }}
            />
          )}
          {bottom === "effect" && (slot ? <EffectEditor state={state} slot={slot} /> : <div className="muted">Choose or add an effect in the mixer.</div>)}
          {bottom === "inspector" && <Inspector state={state} clipId={clip} playhead={playhead} selection={selection} />}
          {bottom === "notes" && (
            <textarea
              className="textarea"
              style={{ width: "100%", height: "100%" }}
              defaultValue={p.notes}
              key={p.id}
              placeholder="Lyrics ideas, mix notes, what to try next time…"
              onBlur={(e) => e.target.value !== p.notes && void edit({ op: "setNotes", notes: e.target.value })}
            />
          )}
        </div>
      </div>

      {exporting && <ExportDialog state={state} selection={selection} onClose={() => setExporting(false)} />}
      {revisions && (
        <Modal title="Saved versions" onClose={() => setRevisions(null)} actions={<Button onClick={() => api.studioBackup().then((path) => toast(`Backup written to ${path}`, "ok")).catch(error)}>Make a full backup copy</Button>}>
          <div className="muted" style={{ marginBottom: 8 }}>Every save and autosave is kept (most recent 30 saves and 20 autosaves). Loading one can be undone.</div>
          <div className="list" style={{ maxHeight: 360, overflow: "auto" }}>
            {revisions.map((r) => (
              <div key={r.id} className="list__item">
                <span className="grow">{new Date(r.createdAt).toLocaleString()}</span>
                <span className="tag">{r.kind}</span>
                <Button size="sm" onClick={() => void apply(api.studioLoadRevision(r.id)).then(() => setRevisions(null))}>Load</Button>
              </div>
            ))}
          </div>
        </Modal>
      )}
      {confirmClose && (
        <Confirm
          title="Save before closing?"
          confirmLabel="Save and close"
          onCancel={() => setConfirmClose(false)}
          onConfirm={() => {
            setConfirmClose(false);
            api.studioClose(true).then(() => setStudio(null)).catch(error);
          }}
        >
          You have unsaved changes. Choose “Save and close”, or press Cancel to keep working. (If you close another way, the changes are
          still kept as an autosave.)
        </Confirm>
      )}
    </div>
  );
}
