import { open } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, FileAudio, Mic, Music, Play, Plus, Repeat, Square, Trash2, Upload } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/client";
import type { LyricLine, PitchFrame, RecordingEntry, SongAttemptResult, SongPhrase, SongPlayback, SongSummary, SongTrack, SongView } from "../api/types";
import { useJob } from "../components/controls";
import { Button, Confirm, Empty, Notice, Panel, ProgressBar, Spinner, Tabs, Tag } from "../components/ui";
import { formatTime, relativeDate } from "../lib/music";
import { useApp } from "../state/app";
import { useLivePitch } from "../state/polling";
import { LiveTuner } from "../viz/LiveTuner";
import { PitchView } from "../viz/PitchView";
import { FeedbackPanel } from "./Analyzer";

const SPEEDS = [1, 0.9, 0.8, 0.75, 0.7, 0.6, 0.5];
const newId = () => (typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random()}`);

/** Song time (s) of the current preview, or null when nothing plays. */
function useSongTime(pb: SongPlayback | null) {
  const status = useApp((s) => s.status);
  if (!pb || !status?.previewPlaying) return null;
  return (status.previewPosition / pb.rate) * pb.speed;
}

function lineEnd(lines: LyricLine[], i: number): number | null {
  const l = lines[i]!;
  if (l.end != null) return l.end;
  const next = lines.slice(i + 1).find((x) => x.start != null);
  return next?.start ?? null;
}

// --------------------------------------------------------------- list

function SongList({ onOpen }: { onOpen: (id: string) => void }) {
  const error = useApp((s) => s.error);
  const [songs, setSongs] = useState<SongSummary[] | null>(null);
  const [title, setTitle] = useState("");
  const [artist, setArtist] = useState("");
  useEffect(() => {
    api.songs().then(setSongs).catch(error);
  }, [error]);
  const create = async () => {
    try {
      const s = await api.songCreate(title, artist);
      onOpen(s.id);
    } catch (e) {
      error(e);
    }
  };
  return (
    <div className="grid grid--sidebar" style={{ alignItems: "start" }}>
      <Panel title="Your songs">
        {!songs ? (
          <Spinner />
        ) : songs.length === 0 ? (
          <Empty icon={<Music size={28} />} title="No songs yet">
            Add a song you want to practise, then import its backing track and lyrics.
          </Empty>
        ) : (
          <div className="col" style={{ gap: 6 }}>
            {songs.map((s) => (
              <button key={s.id} className="list-row" style={{ textAlign: "left" }} onClick={() => onOpen(s.id)}>
                <div className="grow">
                  <div style={{ fontWeight: 600 }}>{s.title}</div>
                  <div className="muted" style={{ fontSize: 12 }}>
                    {s.artist || "—"} · {s.lines} lines · {s.phrases} phrases · {relativeDate(s.updatedAt)}
                  </div>
                </div>
                {s.hasBacking && <Tag>backing</Tag>}
                {s.hasReference && <Tag kind="accent">reference</Tag>}
              </button>
            ))}
          </div>
        )}
      </Panel>
      <div className="col" style={{ gap: 12 }}>
        <Panel title="New song">
          <div className="col" style={{ gap: 8 }}>
            <input className="input" placeholder="Title (e.g. 夜に駆ける)" aria-label="Song title" value={title} onChange={(e) => setTitle(e.target.value)} />
            <input className="input" placeholder="Artist (optional)" aria-label="Artist" value={artist} onChange={(e) => setArtist(e.target.value)} />
            <Button variant="primary" icon={<Plus size={14} />} disabled={!title.trim()} onClick={() => void create()}>
              Add song
            </Button>
          </div>
        </Panel>
        <Notice kind="info">
          Use only material you own or have permission to use (for example a purchased karaoke/instrumental track, or your teacher's recording). Nothing is bundled, and nothing leaves this computer.
        </Notice>
      </div>
    </div>
  );
}

// -------------------------------------------------------------- audio tab

function AudioTab({ song, onChange }: { song: SongView; onChange: (s: SongView) => void }) {
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const [job, setJob] = useState<number | null>(null);
  const info = useJob(job, async (j) => {
    setJob(null);
    if (j.state === "failed") error(j.error);
    if (j.state === "done") toast("Imported", "ok");
    onChange(await api.songGet(song.id));
  });
  const importTrack = async (track: SongTrack) => {
    const path = await open({ filters: [{ name: "WAV audio", extensions: ["wav", "WAV"] }], title: track === "backing" ? "Choose the backing track" : "Choose the reference vocal" });
    if (typeof path !== "string") return;
    try {
      setJob(await api.songImportAudio(song.id, track, path));
    } catch (e) {
      error(e);
    }
  };
  const remove = async (track: SongTrack) => {
    try {
      onChange(await api.songRemoveAudio(song.id, track));
    } catch (e) {
      error(e);
    }
  };
  const row = (track: SongTrack, label: string, hint: string) => {
    const a = track === "backing" ? song.data.backing : song.data.reference;
    return (
      <Panel title={label}>
        <p className="muted" style={{ marginTop: 0, fontSize: 13 }}>{hint}</p>
        {a ? (
          <div className="row" style={{ gap: 8 }}>
            <FileAudio size={16} />
            <span className="grow">
              {a.originalName} · {formatTime(a.frames / a.sampleRate)} · {a.channels === 2 ? "stereo" : "mono"}
            </span>
            <Button size="sm" onClick={() => void importTrack(track)} disabled={job != null}>Replace</Button>
            <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} aria-label={`Remove ${label}`} onClick={() => void remove(track)} />
          </div>
        ) : (
          <Button icon={<Upload size={14} />} onClick={() => void importTrack(track)} disabled={job != null}>
            Import WAV…
          </Button>
        )}
      </Panel>
    );
  };
  return (
    <div className="col" style={{ gap: 12 }}>
      {job != null && <ProgressBar value={info?.progress ?? 0} label={info?.label ?? "Importing"} />}
      {row("backing", "Backing track", "The instrumental you sing along with. WAV files up to 10 minutes; converted for your audio device.")}
      {row("reference", "Reference vocal (optional)", "An isolated vocal you have the rights to use — or your teacher's or your own best take. Attempts are compared with its pitch and timing.")}
      <p className="faint" style={{ fontSize: 12 }}>Only WAV import is supported in this version. Convert other formats to WAV with a free tool first.</p>
    </div>
  );
}

// ------------------------------------------------------------- lyrics tab

function LyricsTab({ song, onChange, playback, setPlayback, speed }: { song: SongView; onChange: (s: SongView) => void; playback: SongPlayback | null; setPlayback: (p: SongPlayback | null) => void; speed: number }) {
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const [lines, setLines] = useState<LyricLine[]>(song.data.lines);
  const [dirty, setDirty] = useState(false);
  const [paste, setPaste] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [tapIndex, setTapIndex] = useState<number | null>(null);
  const songTime = useSongTime(playback);
  const timeRef = useRef<number | null>(null);
  timeRef.current = songTime;

  useEffect(() => {
    setLines(song.data.lines);
    setDirty(false);
  }, [song.data.lines]);

  const set = (i: number, patch: Partial<LyricLine>) => {
    setLines((ls) => ls.map((l, k) => (k === i ? { ...l, ...patch } : l)));
    setDirty(true);
  };
  const save = async () => {
    try {
      onChange(await api.songUpdate(song.id, { lines }));
      setDirty(false);
    } catch (e) {
      error(e);
    }
  };
  const replaceFromText = async (text: string) => {
    try {
      onChange(await api.songImportLyrics(song.id, text));
      setPaste("");
      toast("Lyrics imported", "ok");
    } catch (e) {
      error(e);
    }
  };
  const importFile = async () => {
    const path = await open({ filters: [{ name: "Lyrics", extensions: ["lrc", "txt", "LRC", "TXT"] }], title: "Choose a lyrics file (UTF-8 .lrc or .txt)" });
    if (typeof path !== "string") return;
    try {
      onChange(await api.songImportLyricsFile(song.id, path));
      toast("Lyrics imported", "ok");
    } catch (e) {
      error(e);
    }
  };

  // tap timing: Space marks the start of the next line while the backing plays
  const startTapping = async () => {
    const first = Math.max(0, lines.findIndex((l) => l.start == null));
    const from = first > 0 ? (lines[first - 1]!.start ?? 0) : 0;
    try {
      setPlayback(await api.songPlay(song.id, song.data.backing ? "backing" : "reference", from, null, false, speed));
      setTapIndex(first);
    } catch (e) {
      error(e);
    }
  };
  const stopTapping = useCallback(async () => {
    setTapIndex(null);
    await api.songStop().catch(() => undefined);
    setPlayback(null);
  }, [setPlayback]);
  useEffect(() => {
    if (tapIndex == null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "Space" || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      e.preventDefault();
      const t = timeRef.current;
      if (t == null) return;
      setLines((ls) =>
        ls.map((l, k) => {
          if (k === tapIndex) return { ...l, start: Math.round(t * 100) / 100 };
          if (k === tapIndex - 1 && l.end == null) return { ...l, end: Math.round(t * 100) / 100 };
          return l;
        }),
      );
      setDirty(true);
      if (tapIndex + 1 >= lines.length) void stopTapping();
      else setTapIndex(tapIndex + 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tapIndex, lines.length, stopTapping]);

  const makePhrase = async () => {
    const idx = lines.map((l, i) => (selected.has(l.id) ? i : -1)).filter((i) => i >= 0);
    if (!idx.length) return;
    const start = lines[idx[0]!]!.start;
    const end = lineEnd(lines, idx[idx.length - 1]!);
    if (start == null || end == null) {
      error(new Error("Set start times for the selected lines (and the line after them) first."));
      return;
    }
    const phrase: SongPhrase = { id: newId(), name: `Phrase ${song.data.phrases.length + 1}`, start, end, lineIds: idx.map((i) => lines[i]!.id), notes: "" };
    try {
      onChange(await api.songUpdate(song.id, { lines, phrases: [...song.data.phrases, phrase] }));
      setSelected(new Set());
      setDirty(false);
      toast(`${phrase.name} added — practise it in the Practice tab`, "ok");
    } catch (e) {
      error(e);
    }
  };

  const num = (v: string) => (v.trim() === "" ? null : Number.isFinite(Number(v)) ? Number(v) : null);

  return (
    <div className="col" style={{ gap: 12 }}>
      <div className="row row--wrap" style={{ gap: 8 }}>
        <Button icon={<Upload size={14} />} onClick={() => void importFile()}>Import .lrc / .txt…</Button>
        {tapIndex == null ? (
          <Button disabled={!lines.length || (!song.data.backing && !song.data.reference) || !song.readySpeeds.includes(speed)} onClick={() => void startTapping()} title="Play the song and press Space at the start of each line">
            Tap timing…
          </Button>
        ) : (
          <Button variant="danger" icon={<Square size={13} />} onClick={() => void stopTapping()}>Stop tapping</Button>
        )}
        <Button disabled={!selected.size} onClick={() => void makePhrase()}>Make phrase from selected</Button>
        <div className="grow" />
        {dirty && (
          <>
            <Button onClick={() => { setLines(song.data.lines); setDirty(false); }}>Discard</Button>
            <Button variant="primary" onClick={() => void save()}>Save changes</Button>
          </>
        )}
      </div>
      {tapIndex != null && (
        <Notice kind="info">
          Tap timing: press <b>Space</b> when line {tapIndex + 1} (“{lines[tapIndex]?.text}”) starts. {songTime != null ? `Now at ${formatTime(songTime, true)}.` : "Waiting for playback…"}
        </Notice>
      )}
      {lines.length === 0 ? (
        <Panel title="Add lyrics">
          <p className="muted" style={{ marginTop: 0 }}>Paste lyrics (one line per row) or LRC text with [mm:ss.xx] times.</p>
          <textarea className="textarea" rows={8} style={{ width: "100%", fontFamily: "var(--font-jp)" }} value={paste} onChange={(e) => setPaste(e.target.value)} aria-label="Lyrics text" />
          <Button variant="primary" disabled={!paste.trim()} onClick={() => void replaceFromText(paste)} style={{ marginTop: 8 }}>
            Use these lyrics
          </Button>
        </Panel>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <table className="table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th style={{ width: 24 }} />
                <th>Lyric</th>
                <th>Reading (kana)</th>
                <th>Romaji</th>
                <th style={{ width: 50 }}>拍</th>
                <th style={{ width: 80 }}>Start (s)</th>
                <th style={{ width: 80 }}>End (s)</th>
              </tr>
            </thead>
            <tbody>
              {lines.map((l, i) => {
                const r = song.romaji[i];
                const active = songTime != null && l.start != null && songTime >= l.start && songTime < (lineEnd(lines, i) ?? Infinity);
                return (
                  <tr key={l.id} style={{ background: active ? "var(--accent-dim)" : tapIndex === i ? "var(--raised)" : undefined }}>
                    <td>
                      <input type="checkbox" aria-label={`Select line ${i + 1}`} checked={selected.has(l.id)} onChange={(e) => setSelected((s) => { const n = new Set(s); if (e.target.checked) n.add(l.id); else n.delete(l.id); return n; })} />
                    </td>
                    <td><input className="input" style={{ width: "100%", fontFamily: "var(--font-jp)" }} aria-label={`Line ${i + 1} lyric`} value={l.text} onChange={(e) => set(i, { text: e.target.value })} /></td>
                    <td><input className="input" style={{ width: "100%", fontFamily: "var(--font-jp)" }} aria-label={`Line ${i + 1} reading`} placeholder={r?.hasKanji ? "add kana reading" : ""} value={l.reading} onChange={(e) => set(i, { reading: e.target.value })} /></td>
                    <td className="muted" style={{ fontSize: 12 }}>{dirty ? "…" : r?.romaji}{!dirty && r?.hasKanji && <Tag kind="warn">needs reading</Tag>}</td>
                    <td className="mono">{dirty ? "" : (r?.morae ?? "")}</td>
                    <td><input className="input mono" style={{ width: 72 }} aria-label={`Line ${i + 1} start`} value={l.start ?? ""} onChange={(e) => set(i, { start: num(e.target.value) })} /></td>
                    <td><input className="input mono" style={{ width: 72 }} aria-label={`Line ${i + 1} end`} value={l.end ?? ""} placeholder={lineEnd(lines, i)?.toString() ?? ""} onChange={(e) => set(i, { end: num(e.target.value) })} /></td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {lines.length > 0 && (
        <details>
          <summary className="muted">Replace all lyrics with pasted text…</summary>
          <textarea className="textarea" rows={6} style={{ width: "100%", marginTop: 8, fontFamily: "var(--font-jp)" }} value={paste} onChange={(e) => setPaste(e.target.value)} aria-label="Replacement lyrics text" />
          <Button disabled={!paste.trim()} onClick={() => void replaceFromText(paste)} style={{ marginTop: 6 }}>Replace lyrics</Button>
        </details>
      )}
    </div>
  );
}

// ----------------------------------------------------------- practice tab

function shift(frames: PitchFrame[], by: number): PitchFrame[] {
  return frames.map((f) => ({ ...f, time: f.time - by }));
}

function PracticeTab({ song, onChange, playback, setPlayback, speed, setSpeed }: { song: SongView; onChange: (s: SongView) => void; playback: SongPlayback | null; setPlayback: (p: SongPlayback | null) => void; speed: number; setSpeed: (s: number) => void }) {
  const settings = useApp((s) => s.settings);
  const capture = useApp((s) => s.capture);
  const error = useApp((s) => s.error);
  const go = useApp((s) => s.go);
  const live = useLivePitch(true);
  const [phraseId, setPhraseId] = useState<string | null>(song.data.phrases[0]?.id ?? null);
  const [looped, setLooped] = useState(true);
  const [withBacking, setWithBacking] = useState(true);
  const [result, setResult] = useState<SongAttemptResult | null>(null);
  const [reference, setReference] = useState<PitchFrame[] | null>(null);
  const [prepJob, setPrepJob] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDel, setConfirmDel] = useState<SongPhrase | null>(null);
  const phrase = song.data.phrases.find((p) => p.id === phraseId) ?? null;
  const ready = song.readySpeeds.includes(speed);
  const recording = !!capture?.active && (capture.kind ?? "").startsWith("song:");
  const songTime = useSongTime(playback);

  const prep = useJob(prepJob, async (j) => {
    setPrepJob(null);
    if (j.state === "failed") error(j.error);
    onChange(await api.songGet(song.id));
  });

  // keyed on values, not object identity: the song object is replaced after every save
  const refKey = song.data.reference ? `${song.data.reference.file}:${song.data.reference.frames}` : "";
  const phraseKey = phrase ? `${phrase.id}:${phrase.start}:${phrase.end}` : "";
  useEffect(() => {
    setResult(null);
    setReference(null);
    const [pid] = phraseKey.split(":");
    if (pid && refKey) api.songReferenceContour(song.id, pid).then(setReference).catch(() => setReference(null));
  }, [phraseKey, refKey, song.id]);

  const play = async (track: SongTrack) => {
    if (!phrase) return;
    try {
      setPlayback(await api.songPlay(song.id, track, phrase.start, phrase.end, looped, speed));
    } catch (e) {
      error(e);
    }
  };
  const stop = async () => {
    await api.songStop().catch(() => undefined);
    setPlayback(null);
  };
  const toggleAttempt = async () => {
    if (!phrase) return;
    setBusy(true);
    try {
      if (recording) {
        const r = await api.songAttemptStop();
        setPlayback(null);
        setResult(r);
        onChange(await api.songGet(song.id));
      } else {
        setResult(null);
        setPlayback(await api.songAttemptStart(song.id, phrase.id, speed, withBacking));
      }
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  const updatePhrase = async (patch: Partial<SongPhrase>) => {
    if (!phrase) return;
    try {
      onChange(await api.songUpdate(song.id, { phrases: song.data.phrases.map((p) => (p.id === phrase.id ? { ...p, ...patch } : p)) }));
    } catch (e) {
      error(e);
    }
  };
  const openAttempt = async (r: RecordingEntry) => {
    try {
      const a = await api.recordingAnalysis(r.id);
      const sp = typeof r.summary.speed === "number" ? (r.summary.speed as number) : 1;
      const start = Math.max(0, (phrase?.start ?? 0) - 1.5);
      setResult({
        recording: a.recording,
        analysis: a.analysis,
        feedback: a.feedback,
        comparison: null,
        contour: a.analysis.frames.map((f) => ({ ...f, time: start + f.time * sp })),
        notes: ["Reopened take: the reference comparison is shown when you record a new attempt."],
      });
    } catch (e) {
      error(e);
    }
  };

  if (!song.data.phrases.length) {
    return (
      <Empty icon={<Repeat size={28} />} title="No phrases yet">
        In <b>Lyrics & timing</b>, set line start times (tap along with the song), tick a few lines and choose <b>Make phrase from selected</b>.
      </Empty>
    );
  }
  const lines = phrase ? song.data.lines.map((l, i) => ({ l, r: song.romaji[i] })).filter(({ l }) => phrase.lineIds.includes(l.id)) : [];
  const attempts = song.attempts.filter((a) => phrase && a.exerciseId === `song:${song.id}:${phrase.id}`);
  const dur = phrase ? phrase.end - phrase.start : 0;
  const cursor = songTime != null && phrase ? songTime - phrase.start : null;
  const c = result?.comparison;

  return (
    <div className="grid grid--sidebar" style={{ alignItems: "start" }}>
      <div className="col" style={{ gap: 12 }}>
        {phrase && (
          <Panel title={phrase.name} actions={<span className="mono faint">{formatTime(phrase.start, true)} – {formatTime(phrase.end, true)}</span>}>
            <div className="col" style={{ gap: 4, marginBottom: 10 }}>
              {lines.map(({ l, r }) => (
                <div key={l.id}>
                  {settings?.showKana !== false && <div style={{ fontFamily: "var(--font-jp)", fontSize: 20 }}>{l.text}</div>}
                  {settings?.showKana !== false && l.reading && <div className="muted" style={{ fontFamily: "var(--font-jp)", fontSize: 13 }}>{l.reading}</div>}
                  {settings?.showRomaji !== false && <div className="muted" style={{ fontSize: 13 }}>{r?.romaji}{r?.morae ? ` · ${r.morae} 拍` : ""}</div>}
                </div>
              ))}
            </div>
            <div className="row row--wrap" style={{ gap: 8, alignItems: "center" }}>
              <label className="row" style={{ gap: 6 }}>
                <span className="muted">Speed</span>
                <select className="select" value={speed} aria-label="Practice speed" onChange={(e) => setSpeed(Number(e.target.value))}>
                  {SPEEDS.map((s) => (
                    <option key={s} value={s}>
                      {Math.round(s * 100)}%{song.readySpeeds.includes(s) ? "" : " (prepare)"}
                    </option>
                  ))}
                </select>
              </label>
              {!ready && (
                <Button size="sm" disabled={prepJob != null} onClick={() => api.songPrepare(song.id, speed).then(setPrepJob).catch(error)}>
                  Prepare {Math.round(speed * 100)}%
                </Button>
              )}
              <label className="row" style={{ gap: 4 }}>
                <input type="checkbox" checked={looped} onChange={(e) => setLooped(e.target.checked)} /> Loop
              </label>
              <Button size="sm" icon={<Play size={13} />} disabled={!ready || !song.data.backing || recording} onClick={() => void play("backing")}>Backing</Button>
              <Button size="sm" icon={<Play size={13} />} disabled={!ready || !song.data.reference || recording} onClick={() => void play("reference")}>Reference</Button>
              <Button size="sm" icon={<Square size={13} />} disabled={!playback || recording} onClick={() => void stop()}>Stop</Button>
            </div>
            {prepJob != null && <div style={{ marginTop: 8 }}><ProgressBar value={prep?.progress ?? 0} label="Slowing down without changing pitch…" /></div>}
          </Panel>
        )}

        <Panel title="Record an attempt">
          {!settings?.microphoneEnabled ? (
            <Notice kind="warn">
              Microphone access is off. <Button size="sm" onClick={() => go("settings")}>Enable in Settings</Button>
            </Notice>
          ) : (
            <div className="col" style={{ gap: 8 }}>
              <div className="row row--wrap" style={{ gap: 10, alignItems: "center" }}>
                <Button variant={recording ? "danger" : "rec"} icon={recording ? <Square size={14} /> : <Mic size={14} />} disabled={busy || !phrase || (withBacking && !!song.data.backing && !ready)} onClick={() => void toggleAttempt()}>
                  {recording ? "Stop and analyse" : "Record attempt"}
                </Button>
                <label className="row" style={{ gap: 4 }}>
                  <input type="checkbox" checked={withBacking} disabled={!song.data.backing || recording} onChange={(e) => setWithBacking(e.target.checked)} /> Play backing (use headphones)
                </label>
                {recording && <span className="mono">{capture ? formatTime(capture.seconds) : ""}</span>}
              </div>
              {recording && <LiveTuner framesRef={live} />}
              <p className="faint" style={{ margin: 0, fontSize: 12 }}>
                The backing starts 1.5 s before the phrase. Sing comfortably — accuracy and timing matter, not volume. Stop if anything hurts.
              </p>
            </div>
          )}
        </Panel>

        {result && phrase && (
          <>
            <Panel title="Pitch: your attempt vs reference">
              <PitchView frames={shift(result.contour.filter((f) => f.time >= phrase.start - 0.3 && f.time <= phrase.end + 0.3), phrase.start)} compare={reference ? shift(reference, phrase.start) : undefined} duration={dur} cursor={cursor} height={220} />
              <div className="row row--wrap" style={{ gap: 16, marginTop: 8 }}>
                {c ? (
                  <>
                    <span>Pitch vs reference: <b className="mono">{c.meanAbsPitchDiffCents != null ? `${c.meanAbsPitchDiffCents.toFixed(0)}¢` : "—"}</b> average difference{c.medianPitchDiffCents != null && Math.abs(c.medianPitchDiffCents) > 15 ? ` (mostly ${c.medianPitchDiffCents > 0 ? "sharp" : "flat"})` : ""}</span>
                    <span>Timing: <b className="mono">{Math.abs(c.medianTimingOffsetMs).toFixed(0)} ms</b> {c.medianTimingOffsetMs > 0 ? "late" : "early"} (median)</span>
                    <span>Voiced overlap: <b className="mono">{Math.round(c.voicedOverlap * 100)}%</b></span>
                  </>
                ) : (
                  <span className="muted">No reference comparison for this take.</span>
                )}
              </div>
              {result.notes.map((n) => (
                <p key={n} className="faint" style={{ margin: "6px 0 0", fontSize: 12 }}>{n}</p>
              ))}
            </Panel>
            <FeedbackPanel fb={result.feedback} title="Measured feedback" />
          </>
        )}
      </div>

      <div className="col" style={{ gap: 12 }}>
        <Panel title="Phrases">
          <div className="col" style={{ gap: 4 }}>
            {song.data.phrases.map((p) => (
              <button key={p.id} className="list-row" aria-pressed={p.id === phraseId} style={{ textAlign: "left" }} onClick={() => setPhraseId(p.id)}>
                <span className="grow">{p.name}</span>
                <span className="mono faint">{formatTime(p.start)}</span>
              </button>
            ))}
          </div>
        </Panel>
        {phrase && (
          <Panel title="Phrase settings">
            <div className="col" style={{ gap: 6 }}>
              <input className="input" aria-label="Phrase name" defaultValue={phrase.name} key={`n${phrase.id}`} onBlur={(e) => e.target.value.trim() && e.target.value !== phrase.name && void updatePhrase({ name: e.target.value.trim() })} />
              <div className="row" style={{ gap: 6 }}>
                <label className="col grow" style={{ fontSize: 12 }}>
                  Start (s)
                  <input className="input mono" defaultValue={phrase.start} key={`s${phrase.id}${phrase.start}`} onBlur={(e) => Number(e.target.value) !== phrase.start && void updatePhrase({ start: Number(e.target.value) })} />
                </label>
                <label className="col grow" style={{ fontSize: 12 }}>
                  End (s)
                  <input className="input mono" defaultValue={phrase.end} key={`e${phrase.id}${phrase.end}`} onBlur={(e) => Number(e.target.value) !== phrase.end && void updatePhrase({ end: Number(e.target.value) })} />
                </label>
              </div>
              <textarea className="textarea" rows={3} placeholder="Notes for this phrase (breath marks, tricky words…)" aria-label="Phrase notes" defaultValue={phrase.notes} key={`t${phrase.id}`} onBlur={(e) => e.target.value !== phrase.notes && void updatePhrase({ notes: e.target.value })} />
              <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} onClick={() => setConfirmDel(phrase)}>Delete phrase</Button>
            </div>
          </Panel>
        )}
        <Panel title={`Attempts (${attempts.length})`}>
          {attempts.length === 0 ? (
            <div className="muted">No attempts at this phrase yet.</div>
          ) : (
            <div className="col" style={{ gap: 4 }}>
              {attempts.map((a) => (
                <div key={a.id} className="row" style={{ gap: 6 }}>
                  <button className="list-row grow" style={{ textAlign: "left" }} onClick={() => void openAttempt(a)}>
                    <span className="grow" style={{ fontSize: 12 }}>{a.title.split(" · ").slice(-1)[0]} · {relativeDate(a.createdAt)}</span>
                    {typeof a.summary.refPitchDiffCents === "number" && <span className="mono faint" style={{ fontSize: 11 }}>{(a.summary.refPitchDiffCents as number).toFixed(0)}¢</span>}
                  </button>
                  <Button size="sm" variant="ghost" icon={<Play size={12} />} aria-label={`Play ${a.title}`} onClick={() => api.previewFile(a.path).catch(error)} />
                </div>
              ))}
            </div>
          )}
        </Panel>
      </div>
      {confirmDel && (
        <Confirm
          title="Delete phrase?"
          danger
          confirmLabel="Delete"
          onCancel={() => setConfirmDel(null)}
          onConfirm={async () => {
            const id = confirmDel.id;
            setConfirmDel(null);
            try {
              onChange(await api.songUpdate(song.id, { phrases: song.data.phrases.filter((p) => p.id !== id) }));
              setPhraseId(song.data.phrases.find((p) => p.id !== id)?.id ?? null);
            } catch (e) {
              error(e);
            }
          }}
        >
          The phrase is removed; its recorded attempts stay in your library.
        </Confirm>
      )}
    </div>
  );
}

// ------------------------------------------------------------- song page

function SongPage({ id, onBack }: { id: string; onBack: () => void }) {
  const error = useApp((s) => s.error);
  const [song, setSong] = useState<SongView | null>(null);
  const [tab, setTab] = useState<"practice" | "lyrics" | "audio" | "notes">("practice");
  const [playback, setPlayback] = useState<SongPlayback | null>(null);
  const [speed, setSpeedState] = useState(1);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editing, setEditing] = useState(false);
  const status = useApp((s) => s.status);

  useEffect(() => {
    api
      .songGet(id)
      .then((s) => {
        setSong(s);
        setSpeedState(s.data.practiceSpeed || 1);
        if (!s.data.backing && !s.data.reference) setTab("audio");
        else if (!s.data.phrases.length) setTab("lyrics");
      })
      .catch(error);
    return () => {
      void api.songStop().catch(() => undefined);
    };
  }, [id, error]);
  // playback ended on its own
  useEffect(() => {
    if (playback && status && !status.previewPlaying) setPlayback(null);
  }, [status, playback]);

  const setSpeed = (s: number) => {
    setSpeedState(s);
    if (song) api.songUpdate(song.id, { practiceSpeed: s }).then(setSong).catch(error);
  };
  const tabs = useMemo(
    () => [
      { id: "practice" as const, label: "Practice" },
      { id: "lyrics" as const, label: "Lyrics & timing" },
      { id: "audio" as const, label: "Audio" },
      { id: "notes" as const, label: "Notes" },
    ],
    [],
  );
  if (!song) return <Spinner />;
  return (
    <>
      <div className="page-header">
        <div className="row" style={{ gap: 10, alignItems: "center" }}>
          <Button variant="ghost" icon={<ArrowLeft size={16} />} aria-label="Back to songs" onClick={onBack} />
          {editing ? (
            <div className="row" style={{ gap: 6 }}>
              <input className="input" aria-label="Song title" defaultValue={song.title} id="song-title" style={{ fontFamily: "var(--font-jp)" }} />
              <input className="input" aria-label="Artist" defaultValue={song.artist} id="song-artist" />
              <Button
                size="sm"
                variant="primary"
                onClick={() => {
                  const title = (document.getElementById("song-title") as HTMLInputElement).value;
                  const artist = (document.getElementById("song-artist") as HTMLInputElement).value;
                  api
                    .songUpdate(song.id, { title, artist })
                    .then((s) => {
                      setSong(s);
                      setEditing(false);
                    })
                    .catch(error);
                }}
              >
                Save
              </Button>
              <Button size="sm" onClick={() => setEditing(false)}>Cancel</Button>
            </div>
          ) : (
            <div>
              <h1 style={{ fontFamily: "var(--font-jp)" }}>{song.title}</h1>
              <p>
                {song.artist || "Song Lab"}{" "}
                <button className="btn btn--ghost btn--sm" onClick={() => setEditing(true)}>
                  Rename
                </button>
              </p>
            </div>
          )}
        </div>
        <Button variant="ghost" icon={<Trash2 size={14} />} onClick={() => setConfirmDelete(true)}>Delete song</Button>
      </div>
      <Tabs tabs={tabs} value={tab} onChange={setTab} />
      <div style={{ marginTop: 12 }}>
        {tab === "practice" && <PracticeTab song={song} onChange={setSong} playback={playback} setPlayback={setPlayback} speed={speed} setSpeed={setSpeed} />}
        {tab === "lyrics" && <LyricsTab song={song} onChange={setSong} playback={playback} setPlayback={setPlayback} speed={song.readySpeeds.includes(speed) ? speed : 1} />}
        {tab === "audio" && <AudioTab song={song} onChange={setSong} />}
        {tab === "notes" && (
          <Panel title="Notes">
            <textarea
              className="input"
              rows={12}
              style={{ width: "100%", fontFamily: "var(--font-jp)" }}
              aria-label="Song notes"
              placeholder="Breathing spots, words to practise, what your teacher said…"
              defaultValue={song.data.notes}
              onBlur={(e) => e.target.value !== song.data.notes && api.songUpdate(song.id, { notes: e.target.value }).then(setSong).catch(error)}
            />
          </Panel>
        )}
      </div>
      {confirmDelete && (
        <Confirm
          title={`Delete “${song.title}”?`}
          danger
          confirmLabel="Delete song"
          onCancel={() => setConfirmDelete(false)}
          onConfirm={async () => {
            setConfirmDelete(false);
            try {
              await api.songDelete(song.id);
              onBack();
            } catch (e) {
              error(e);
            }
          }}
        >
          This deletes the imported audio, lyrics, phrases and all recorded attempts for this song. Your original files are not touched.
        </Confirm>
      )}
    </>
  );
}

export function SongLab() {
  const [open, setOpen] = useState<string | null>(null);
  return (
    <div className="workspace">
      {open ? (
        <SongPage id={open} onBack={() => setOpen(null)} />
      ) : (
        <>
          <div className="page-header">
            <div>
              <h1>Japanese Song Lab</h1>
              <p>Practise real songs phrase by phrase — slowed down without changing pitch, with kana and romaji, and compared with a reference.</p>
            </div>
          </div>
          <SongList onOpen={setOpen} />
        </>
      )}
    </div>
  );
}
