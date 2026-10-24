import { CircleStop, GitCompare, Pause, Play, Trash2, Volume2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import type { AnalyzerResult, Feedback, PhraseAnalysis, RecordingComparison, RecordingEntry } from "../api/types";
import { Button, Confirm, Empty, Notice, Panel, Spinner, Tag } from "../components/ui";
import { formatCents, formatTime, hzToMidi, noteName, relativeDate } from "../lib/music";
import { useApp } from "../state/app";
import { useLivePitch } from "../state/polling";
import { LiveTuner } from "../viz/LiveTuner";
import { Meter } from "../viz/Meter";
import { PitchView } from "../viz/PitchView";

export function FeedbackPanel({ fb, title = "Feedback" }: { fb: Feedback; title?: string }) {
  const openExercise = useApp((s) => s.openExercise);
  return (
    <Panel title={title}>
      <div className="stack">
        <div style={{ fontSize: 16, fontWeight: 650 }}>{fb.headline}</div>
        {fb.uncertainty && <Notice kind="warn">{fb.uncertainty}</Notice>}
        {fb.wentWell.length > 0 && (
          <div>
            <div className="faint" style={{ fontSize: 11, textTransform: "uppercase", letterSpacing: ".08em" }}>
              What went well
            </div>
            <ul style={{ margin: "4px 0 0", paddingLeft: 18 }}>
              {fb.wentWell.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          </div>
        )}
        {fb.primaryOpportunity && (
          <div>
            <div className="faint" style={{ fontSize: 11, textTransform: "uppercase", letterSpacing: ".08em" }}>
              One thing to work on
            </div>
            <div style={{ marginTop: 4 }}>{fb.primaryOpportunity}</div>
          </div>
        )}
        {fb.retryGoal && (
          <div className="row">
            <Tag kind="accent">Goal</Tag>
            <span>{fb.retryGoal}</span>
          </div>
        )}
        {fb.suggestion && (
          <div className="row row--between panel panel--flat">
            <div>
              <div>
                Suggested exercise: <strong>{fb.suggestion.title}</strong>
              </div>
              <div className="faint" style={{ fontSize: 11 }}>
                {fb.suggestion.reason}
              </div>
            </div>
            <Button size="sm" onClick={() => openExercise(fb.suggestion!.exerciseId)}>
              Practise
            </Button>
          </div>
        )}
        {fb.safetyReminder && <div className="faint" style={{ fontSize: 11 }}>{fb.safetyReminder}</div>}
      </div>
    </Panel>
  );
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="panel panel--flat" title={hint}>
      <div className="faint" style={{ fontSize: 11 }}>{label}</div>
      <div className="kpi" style={{ fontSize: 17 }}>{value}</div>
    </div>
  );
}

function regionStats(a: PhraseAnalysis, sel: [number, number]) {
  const m = a.frames.filter((f) => f.hz && f.time >= sel[0] && f.time <= sel[1]).map((f) => hzToMidi(f.hz!));
  if (m.length < 5) return null;
  const sorted = [...m].sort((x, y) => x - y);
  const med = sorted[Math.floor(sorted.length / 2)]!;
  const mean = m.reduce((s, v) => s + v, 0) / m.length;
  const sd = Math.sqrt(m.reduce((s, v) => s + (v - mean) ** 2, 0) / m.length) * 100;
  return { med, sd, voiced: m.length * a.hopSeconds };
}

export function Analyzer() {
  const settings = useApp((s) => s.settings);
  const status = useApp((s) => s.status);
  const capture = useApp((s) => s.capture);
  const error = useApp((s) => s.error);
  const go = useApp((s) => s.go);
  const live = useLivePitch(true);
  const [busy, setBusy] = useState(false);
  const [takes, setTakes] = useState<RecordingEntry[]>([]);
  const [current, setCurrent] = useState<AnalyzerResult | null>(null);
  const [peaks, setPeaks] = useState<number[] | undefined>();
  const [compareWith, setCompareWith] = useState<AnalyzerResult | null>(null);
  const [comparison, setComparison] = useState<RecordingComparison | null>(null);
  const [selection, setSelection] = useState<[number, number] | null>(null);
  const [refNote, setRefNote] = useState<number>(57);
  const [confirmDel, setConfirmDel] = useState<RecordingEntry | null>(null);
  const recording = capture?.active && capture.kind === "analyzer";
  const otherCapture = capture?.active && capture.kind !== "analyzer";

  const loadTakes = useCallback(() => {
    api.recordings("analyzer", 100).then(setTakes).catch(error);
  }, [error]);
  useEffect(loadTakes, [loadTakes]);
  useEffect(() => {
    if (settings) setRefNote(Math.round((settings.voiceRange.low + settings.voiceRange.high) / 2));
  }, [settings]);

  const show = useCallback(
    async (r: AnalyzerResult) => {
      setCurrent(r);
      setSelection(null);
      setComparison(null);
      setCompareWith(null);
      setPeaks(undefined);
      api.waveform(r.recording.path, 0, null, 1200).then(setPeaks).catch(() => setPeaks(undefined));
    },
    [],
  );

  const toggleRecord = async () => {
    setBusy(true);
    try {
      if (recording) {
        const r = await api.analyzerStop();
        await show(r);
        loadTakes();
      } else {
        await api.stopPreview().catch(() => undefined);
        await api.analyzerStart();
      }
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code === "Space" && !(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement)) {
        e.preventDefault();
        if (!busy && !otherCapture) void toggleRecord();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const openTake = async (t: RecordingEntry) => {
    try {
      await show(await api.recordingAnalysis(t.id));
    } catch (e) {
      error(e);
    }
  };

  const compare = async (t: RecordingEntry) => {
    if (!current) return;
    try {
      const [other, cmp] = await Promise.all([api.recordingAnalysis(t.id), api.compareRecordings(t.id, current.recording.id)]);
      setCompareWith(other);
      setComparison(cmp);
    } catch (e) {
      error(e);
    }
  };

  const a = current?.analysis;
  const previewing = status?.previewPlaying;
  const cursor = previewing && a ? status!.previewPosition / a.sampleRate : null;
  const inputMeters = status?.inputs.slice(settings?.inputChannel ?? 0, (settings?.inputChannel ?? 0) + 1) ?? [];
  const clipped = inputMeters.some((m) => m.clipped);
  const region = useMemo(() => (a && selection ? regionStats(a, selection) : null), [a, selection]);

  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Vocal Analyzer</h1>
          <p>Sing a phrase, then see your pitch, steadiness and dynamics — measured, not guessed.</p>
        </div>
      </div>
      {!settings?.microphoneEnabled && (
        <Notice kind="warn">
          Microphone access is off. <Button size="sm" onClick={() => go("settings")}>Enable in Settings</Button>
        </Notice>
      )}
      <div className="grid grid--sidebar" style={{ marginTop: 12 }}>
        <div className="grid" style={{ alignContent: "start" }}>
          <Panel title="Live input">
            <div className="row" style={{ alignItems: "center", gap: 16 }}>
              <div className="col" style={{ alignItems: "center" }}>
                <Meter values={inputMeters.length ? inputMeters : [{ peak: 0, rms: 0, clipped: false }]} length={120} thickness={10} onResetClip={() => void api.resetClips()} label="Microphone" />
                <span className="faint" style={{ fontSize: 10 }}>input</span>
              </div>
              <div className="grow">
                <LiveTuner framesRef={live} />
              </div>
            </div>
            {clipped && (
              <div style={{ marginTop: 10 }}>
                <Notice kind="danger" onClose={() => void api.resetClips()}>
                  The input clipped. Lower the gain on your interface or move a little further from the mic.
                </Notice>
              </div>
            )}
            <div className="row" style={{ marginTop: 14 }}>
              <Button
                variant={recording ? "default" : "rec"}
                size="lg"
                disabled={busy || !settings?.microphoneEnabled || !!otherCapture}
                icon={recording ? <CircleStop size={18} /> : <span className="dot" style={{ background: "#fff" }} />}
                onClick={() => void toggleRecord()}
                aria-label={recording ? "Stop recording" : "Record a phrase"}
              >
                {recording ? `Stop  ${formatTime(capture?.seconds ?? 0)}` : "Record"}
              </Button>
              <kbd>Space</kbd>
              {busy && <Spinner label={recording ? "Analysing…" : undefined} />}
              <span className="grow" />
              <select className="select" value={refNote} onChange={(e) => setRefNote(Number(e.target.value))} aria-label="Reference note">
                {Array.from({ length: 36 }, (_, i) => 40 + i).map((m) => (
                  <option key={m} value={m}>
                    {noteName(m)}
                  </option>
                ))}
              </select>
              <Button icon={<Volume2 size={16} />} onClick={() => api.playReference(refNote, 2).catch(error)}>
                Play note
              </Button>
            </div>
          </Panel>

          {!a ? (
            <Panel>
              <Empty title="No take selected">Record a phrase or open a take from the list to see its analysis.</Empty>
            </Panel>
          ) : (
            <Panel
              title={
                <span>
                  {current!.recording.title} · {formatTime(a.duration, true)}
                </span>
              }
              actions={
                <>
                  <Tag kind={a.quality.level === "good" ? "ok" : a.quality.level === "fair" ? "warn" : "danger"}>
                    recording quality: {a.quality.level}
                  </Tag>
                  {previewing ? (
                    <Button size="sm" icon={<Pause size={14} />} onClick={() => api.stopPreview().catch(error)}>
                      Stop
                    </Button>
                  ) : (
                    <Button size="sm" icon={<Play size={14} />} onClick={() => api.previewFile(current!.recording.path, selection?.[0] ?? 0).catch(error)}>
                      Play
                    </Button>
                  )}
                </>
              }
            >
              <PitchView
                frames={a.frames}
                duration={a.duration}
                peaks={peaks}
                notes={a.notes}
                compare={compareWith?.analysis.frames}
                cursor={cursor}
                selection={selection}
                onSelect={setSelection}
                onSeek={(t) => api.previewFile(current!.recording.path, t).catch(error)}
                height={320}
              />
              <div className="faint" style={{ fontSize: 11, marginTop: 6 }}>
                Blue line = your pitch (fainter where the measurement is less certain; gaps = no clear pitch). Drag to select a region.
                {compareWith && " Grey line = compared take."}
              </div>
              {region && selection && (
                <div className="row" style={{ marginTop: 8 }}>
                  <Tag kind="accent">
                    Selection {selection[0].toFixed(2)}–{selection[1].toFixed(2)} s
                  </Tag>
                  <span>
                    median {noteName(region.med)} {formatCents((region.med - Math.round(region.med)) * 100)} · spread ±{region.sd.toFixed(0)}¢ ·{" "}
                    {region.voiced.toFixed(1)} s voiced
                  </span>
                </div>
              )}
              <div className="grid grid--3" style={{ marginTop: 14 }}>
                <Metric
                  label="Range used"
                  value={a.pitch.lowMidi != null && a.pitch.highMidi != null ? `${noteName(a.pitch.lowMidi)} – ${noteName(a.pitch.highMidi)}` : "—"}
                  hint="5th–95th percentile of measured pitch"
                />
                <Metric
                  label="Longest note steadiness"
                  value={a.sustain ? `±${a.sustain.wobbleCents.toFixed(0)}¢ · drift ${formatCents(a.sustain.driftCents)}` : "—"}
                  hint="Wobble around the trend and total drift over the longest held note"
                />
                <Metric label="Vibrato" value={a.sustain?.vibrato ? `${a.sustain.vibrato.rateHz.toFixed(1)} Hz · ±${a.sustain.vibrato.extentCents.toFixed(0)}¢` : "none detected"} />
                <Metric
                  label="Dynamic range"
                  value={a.dynamics.dynamicRangeDb != null ? `${a.dynamics.dynamicRangeDb.toFixed(1)} dB` : "—"}
                  hint="Difference between the louder and softer sung parts. Not a score — expression is a choice."
                />
                <Metric
                  label="Brightness"
                  value={a.spectral.medianCentroidHz != null ? `${Math.round(a.spectral.medianCentroidHz)} Hz` : "—"}
                  hint="Spectral centroid. Depends on mic and room; compare only takes recorded the same way."
                />
                <Metric
                  label="Measurement confidence"
                  value={a.pitch.meanConfidence != null ? `${Math.round(a.pitch.meanConfidence * 100)}%` : "—"}
                  hint={a.quality.snrDb != null ? `Signal-to-noise ≈ ${a.quality.snrDb.toFixed(0)} dB` : "Background noise unknown (no silence recorded)"}
                />
              </div>
            </Panel>
          )}

          {comparison && (
            <Panel title={`Comparison: ${comparison.a.title} → ${comparison.b.title}`} actions={<Button size="sm" variant="ghost" onClick={() => { setComparison(null); setCompareWith(null); }}>Close</Button>}>
              {comparison.contour && (
                <div className="row row--wrap" style={{ marginBottom: 10 }}>
                  <Tag>Pitch difference after alignment: {comparison.contour.meanAbsPitchDiffCents?.toFixed(0) ?? "—"}¢</Tag>
                  <Tag>Timing offset: {comparison.contour.medianTimingOffsetMs.toFixed(0)} ms</Tag>
                  <Tag>Overlap {(comparison.contour.voicedOverlap * 100).toFixed(0)}%</Tag>
                </div>
              )}
              <table className="table">
                <thead>
                  <tr>
                    <th>Measurement</th>
                    <th>{comparison.a.title}</th>
                    <th>{comparison.b.title}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {comparison.metrics.map((m) => (
                    <tr key={m.key}>
                      <td>{m.label}</td>
                      <td className="mono">{m.a != null ? m.a.toFixed(m.key === "median" ? 1 : 0) : "—"}</td>
                      <td className="mono">{m.b != null ? m.b.toFixed(m.key === "median" ? 1 : 0) : "—"}</td>
                      <td>{m.verdict === "better" ? <Tag kind="ok">steadier</Tag> : m.verdict === "worse" ? <Tag kind="warn">less steady</Tag> : null}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {comparison.notes.map((n) => (
                <div key={n} className="faint" style={{ fontSize: 11, marginTop: 6 }}>
                  {n}
                </div>
              ))}
            </Panel>
          )}
        </div>

        <div className="grid" style={{ alignContent: "start" }}>
          {current && <FeedbackPanel fb={current.feedback} />}
          <Panel title={`Takes (${takes.length})`}>
            {takes.length === 0 ? (
              <div className="muted">Your analyzer takes will be listed here.</div>
            ) : (
              <div className="list" style={{ maxHeight: 420, overflow: "auto" }}>
                {takes.map((t) => (
                  <div key={t.id} className="list__item" aria-selected={current?.recording.id === t.id} onClick={() => void openTake(t)}>
                    <div className="grow">
                      <div>{t.title}</div>
                      <div className="faint" style={{ fontSize: 11 }}>
                        {formatTime(t.duration)} · {relativeDate(t.createdAt)}
                      </div>
                    </div>
                    {current && current.recording.id !== t.id && (
                      <Button size="sm" variant="ghost" icon={<GitCompare size={14} />} aria-label={`Compare with ${t.title}`} title="Compare with the selected take" onClick={(e) => { e.stopPropagation(); void compare(t); }} />
                    )}
                    <Button size="sm" variant="ghost" icon={<Trash2 size={14} />} aria-label={`Delete ${t.title}`} onClick={(e) => { e.stopPropagation(); setConfirmDel(t); }} />
                  </div>
                ))}
              </div>
            )}
          </Panel>
        </div>
      </div>
      {confirmDel && (
        <Confirm
          title="Delete this take?"
          danger
          confirmLabel="Delete"
          onCancel={() => setConfirmDel(null)}
          onConfirm={() => {
            const t = confirmDel;
            setConfirmDel(null);
            api
              .deleteRecording(t.id)
              .then(() => {
                if (current?.recording.id === t.id) setCurrent(null);
                loadTakes();
              })
              .catch(error);
          }}
        >
          “{confirmDel.title}” and its audio file will be permanently deleted.
        </Confirm>
      )}
    </div>
  );
}
