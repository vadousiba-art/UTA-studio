import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useState } from "react";
import { api } from "../../api/client";
import type { BitDepth, RenderReport, StudioState } from "../../api/types";
import { useJob } from "../../components/controls";
import { Button, Field, Modal, Notice, ProgressBar, Switch } from "../../components/ui";
import { formatTime } from "../../lib/music";
import { useApp } from "../../state/app";

export function ExportDialog({ state, selection, onClose }: { state: StudioState; selection: [number, number] | null; onClose: () => void }) {
  const error = useApp((s) => s.error);
  const p = state.project;
  const sr = p.sampleRate;
  const len = p.tracks.reduce((m, t) => Math.max(m, ...t.clips.map((c) => c.position + c.length)), 0);
  const [range, setRange] = useState<"all" | "loop" | "selection">("all");
  const [depth, setDepth] = useState<BitDepth>("int24");
  const [rate, setRate] = useState<number | "">("");
  const [channels, setChannels] = useState(2);
  const [tail, setTail] = useState(2);
  const [dither, setDither] = useState(true);
  const [jobId, setJobId] = useState<number | null>(null);
  const [report, setReport] = useState<RenderReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const job = useJob(jobId, (j) => {
    if (j.state === "done") setReport(j.result as RenderReport);
    else if (j.state === "failed") setFailure(j.error);
    else if (j.state === "cancelled") setFailure("Export cancelled.");
  });

  const bounds = (): [number, number] => {
    if (range === "loop" && p.loopRegion) return [p.loopRegion.start, p.loopRegion.end];
    if (range === "selection" && selection) return [Math.round(selection[0] * sr), Math.round(selection[1] * sr)];
    return [0, len];
  };
  const [s, e] = bounds();

  const start = async () => {
    const path = await save({ title: "Export mix", defaultPath: `${p.name}.wav`, filters: [{ name: "WAV audio", extensions: ["wav"] }] });
    if (!path) return;
    setReport(null);
    setFailure(null);
    try {
      setJobId(
        await api.studioExport(path.toLowerCase().endsWith(".wav") ? path : `${path}.wav`, {
          start: s,
          end: e,
          tailSeconds: tail,
          sampleRate: rate === "" ? null : rate,
          depth,
          channels,
          dither: depth === "int16" && dither,
        }),
      );
    } catch (err) {
      error(err);
    }
  };

  const running = job && (job.state === "queued" || job.state === "running");
  return (
    <Modal
      title="Export mix to WAV"
      onClose={() => !running && onClose()}
      actions={
        running ? (
          <Button variant="danger" onClick={() => jobId != null && api.cancelJob(jobId)}>Cancel export</Button>
        ) : (
          <>
            <Button onClick={onClose}>Close</Button>
            <Button variant="primary" disabled={e <= s} onClick={() => void start()}>
              Export…
            </Button>
          </>
        )
      }
    >
      <div className="stack">
        <Field label="Range">
          <select className="select" value={range} onChange={(ev) => setRange(ev.target.value as typeof range)}>
            <option value="all">Whole project ({formatTime(len / sr)})</option>
            <option value="loop" disabled={!p.loopRegion}>Loop region</option>
            <option value="selection" disabled={!selection}>Selection</option>
          </select>
        </Field>
        <div className="grid grid--3">
          <Field label="Format">
            <select className="select" value={depth} onChange={(ev) => setDepth(ev.target.value as BitDepth)}>
              <option value="int16">16-bit PCM</option>
              <option value="int24">24-bit PCM</option>
              <option value="float32">32-bit float</option>
            </select>
          </Field>
          <Field label="Sample rate">
            <select className="select" value={rate} onChange={(ev) => setRate(ev.target.value ? Number(ev.target.value) : "")}>
              <option value="">Project ({sr / 1000} kHz)</option>
              {[44100, 48000].filter((r) => r !== sr).map((r) => (
                <option key={r} value={r}>{r / 1000} kHz</option>
              ))}
            </select>
          </Field>
          <Field label="Channels">
            <select className="select" value={channels} onChange={(ev) => setChannels(Number(ev.target.value))}>
              <option value={2}>Stereo</option>
              <option value={1}>Mono</option>
            </select>
          </Field>
        </div>
        <Field label={`Tail after the end: ${tail} s`} hint="Leaves room for reverb and delay to fade out.">
          <input className="slider" type="range" min={0} max={8} step={0.5} value={tail} onChange={(ev) => setTail(Number(ev.target.value))} />
        </Field>
        {depth === "int16" && <Switch checked={dither} onChange={setDither} label="Dither (recommended for 16-bit)" />}
        {job && running && (
          <div className="col">
            <ProgressBar value={job.progress} label="Export progress" />
            <span className="faint">{Math.round(job.progress * 100)}%</span>
          </div>
        )}
        {failure && <Notice kind="danger">{failure}</Notice>}
        {report && (
          <Notice kind={report.verified && report.clippedSamples === 0 ? "ok" : "warn"}>
            <div>
              Exported {formatTime(report.durationSeconds, true)} · {report.sampleRate / 1000} kHz · {report.channels === 2 ? "stereo" : "mono"} · peak{" "}
              {report.peakDb.toFixed(1)} dBFS · {report.verified ? "file verified" : "verification failed"}
            </div>
            {report.warnings.map((w) => (
              <div key={w}>{w}</div>
            ))}
            <Button size="sm" style={{ marginTop: 6 }} onClick={() => revealItemInDir(report.path).catch(error)}>
              Show in folder
            </Button>
          </Notice>
        )}
      </div>
    </Modal>
  );
}
