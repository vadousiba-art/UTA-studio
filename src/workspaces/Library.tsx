import { Play, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import type { CoachOverview, NoteEntry, RecordingEntry, SkillHistoryPoint } from "../api/types";
import { Button, Confirm, Empty, Panel, SkillLevel, Spinner, Tabs, Tag } from "../components/ui";
import { formatTime, relativeDate } from "../lib/music";
import { useApp } from "../state/app";

function HistoryChart({ points }: { points: SkillHistoryPoint[] }) {
  const w = 560;
  const h = 150;
  if (points.length === 0) return <div className="muted">No attempts for this skill yet.</div>;
  const n = points.length;
  const x = (i: number) => 20 + (n === 1 ? 0.5 : i / (n - 1)) * (w - 40);
  const y = (score: number) => 10 + (1 - score) * (h - 40);
  const color = (o: string) => (o === "success" ? "#3ccf8e" : o === "partial" ? "#5cb0ff" : o === "needsWork" ? "#f5b041" : "#6f7b88");
  const scored = points.map((p, i) => ({ p, i })).filter((d) => d.p.score != null);
  return (
    <svg width="100%" viewBox={`0 0 ${w} ${h}`} role="img" aria-label="Score history">
      {[0, 0.5, 1].map((v) => (
        <g key={v}>
          <line x1={20} x2={w - 20} y1={y(v)} y2={y(v)} stroke="#222832" />
          <text x={0} y={y(v) + 3} fontSize={9} fill="#6f7b88">{v * 100}</text>
        </g>
      ))}
      {scored.length > 1 && <polyline fill="none" stroke="#3b6f9e" strokeWidth={1.5} points={scored.map((d) => `${x(d.i)},${y(d.p.score!)}`).join(" ")} />}
      {points.map((p, i) => (
        <g key={i}>
          <circle cx={x(i)} cy={p.score != null ? y(p.score) : h - 26} r={4} fill={color(p.outcome)}>
            <title>{`${new Date(p.at).toLocaleString()} · level ${p.level} · ${p.outcome}${p.score != null ? ` · ${(p.score * 100).toFixed(0)}` : ""}`}</title>
          </circle>
          <text x={x(i)} y={h - 6} fontSize={8} fill="#6f7b88" textAnchor="middle">L{p.level}</text>
        </g>
      ))}
    </svg>
  );
}

export function Library() {
  const error = useApp((s) => s.error);
  const toast = useApp((s) => s.toast);
  const info = useApp((s) => s.info);
  const go = useApp((s) => s.go);
  const [tab, setTab] = useState<"progress" | "recordings" | "notes">("progress");
  const [overview, setOverview] = useState<CoachOverview | null>(null);
  const [skill, setSkill] = useState<string | null>(null);
  const [hist, setHist] = useState<SkillHistoryPoint[]>([]);
  const [recs, setRecs] = useState<RecordingEntry[] | null>(null);
  const [query, setQuery] = useState("");
  const [source, setSource] = useState("");
  const [notes, setNotes] = useState<NoteEntry[]>([]);
  const [draft, setDraft] = useState("");
  const [del, setDel] = useState<RecordingEntry | null>(null);

  const loadRecs = useCallback(() => api.recordings(source || undefined, 500).then(setRecs).catch(error), [source, error]);
  useEffect(() => {
    api.coachOverview().then((o) => {
      setOverview(o);
      const first = o.skills.find((s) => s.attempts > 0) ?? o.skills[0];
      if (first) setSkill((s) => s ?? first.id);
    }).catch(error);
    api.notes().then(setNotes).catch(error);
  }, [error]);
  useEffect(() => {
    void loadRecs();
  }, [loadRecs]);
  useEffect(() => {
    if (skill) api.skillHistory(skill).then(setHist).catch(error);
  }, [skill, error]);

  const filtered = useMemo(() => (recs ?? []).filter((r) => !query || r.title.toLowerCase().includes(query.toLowerCase())), [recs, query]);

  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Progress &amp; Library</h1>
          <p>Your practice history, skill progress, recordings and notes — all stored locally.</p>
        </div>
        <div className="mono faint" style={{ fontSize: 11, textAlign: "right" }}>{info?.paths.dataDir}</div>
      </div>
      <Tabs tabs={[{ id: "progress", label: "Skill progress" }, { id: "recordings", label: "Recordings" }, { id: "notes", label: "Training notes" }]} value={tab} onChange={setTab} />
      {tab === "progress" &&
        (!overview ? (
          <Spinner />
        ) : (
          <div className="grid grid--sidebar">
            <Panel title={overview.skills.find((s) => s.id === skill)?.title ?? "Skill"}>
              <HistoryChart points={hist} />
              <div className="faint" style={{ fontSize: 11, marginTop: 4 }}>
                Green = success · blue = partly there · amber = needs work · grey = not measurable (not counted). Levels change only from
                repeated demonstrated results.
              </div>
              <table className="table" style={{ marginTop: 12 }}>
                <thead><tr><th>When</th><th>Exercise</th><th>Level</th><th>Result</th></tr></thead>
                <tbody>
                  {[...hist].reverse().slice(0, 30).map((h, i) => (
                    <tr key={i}>
                      <td>{relativeDate(h.at)}</td>
                      <td>{h.exerciseId}</td>
                      <td>{h.level}</td>
                      <td>{h.outcome}{h.score != null ? ` (${Math.round(h.score * 100)})` : ""}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Panel>
            <Panel title="Skills">
              <div className="list">
                {overview.skills.map((s) => (
                  <div key={s.id} className="list__item" aria-selected={skill === s.id} onClick={() => setSkill(s.id)}>
                    <div className="grow">
                      <div>{s.title}</div>
                      <div className="faint" style={{ fontSize: 10 }}>Stage {s.stage} · {s.attempts} attempts</div>
                    </div>
                    {s.unlocked ? <SkillLevel level={s.level} /> : <Tag>locked</Tag>}
                  </div>
                ))}
              </div>
            </Panel>
          </div>
        ))}
      {tab === "recordings" && (
        <Panel>
          <div className="row" style={{ marginBottom: 10 }}>
            <input className="input grow" type="search" placeholder="Search by title" value={query} onChange={(e) => setQuery(e.target.value)} />
            <select className="select" value={source} onChange={(e) => setSource(e.target.value)} aria-label="Source">
              <option value="">All sources</option>
              <option value="analyzer">Analyzer</option>
              <option value="coach">Coach</option>
              <option value="songlab">Song Lab</option>
            </select>
            <Button onClick={() => go("analyzer")}>Compare in Analyzer</Button>
          </div>
          {!recs ? (
            <Spinner />
          ) : filtered.length === 0 ? (
            <Empty title="No recordings" />
          ) : (
            <table className="table">
              <thead><tr><th>Title</th><th>Source</th><th>Length</th><th>Recorded</th><th /></tr></thead>
              <tbody>
                {filtered.map((r) => (
                  <tr key={r.id}>
                    <td>{r.title}</td>
                    <td><Tag>{r.source}</Tag></td>
                    <td className="mono">{formatTime(r.duration)}</td>
                    <td className="faint">{new Date(r.createdAt).toLocaleString()}</td>
                    <td style={{ textAlign: "right" }}>
                      <Button size="sm" variant="ghost" icon={<Play size={13} />} aria-label={`Play ${r.title}`} onClick={() => api.previewFile(r.path).catch(error)} />
                      <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} aria-label={`Delete ${r.title}`} onClick={() => setDel(r)} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>
      )}
      {tab === "notes" && (
        <div className="grid grid--2">
          <Panel title="New note">
            <textarea className="textarea" style={{ width: "100%" }} placeholder="What felt easier today? What to try next time?" value={draft} onChange={(e) => setDraft(e.target.value)} />
            <div className="row" style={{ marginTop: 8 }}>
              <Button variant="primary" disabled={!draft.trim()} onClick={() => api.addNote(draft).then((n) => { setNotes((x) => [n, ...x]); setDraft(""); toast("Note saved", "ok"); }).catch(error)}>
                Save note
              </Button>
            </div>
          </Panel>
          <Panel title={`Notes (${notes.length})`}>
            {notes.length === 0 ? <div className="muted">No notes yet.</div> : notes.map((n) => (
              <div key={n.id} className="check-row" style={{ alignItems: "flex-start" }}>
                <div className="grow">
                  <div style={{ whiteSpace: "pre-wrap", userSelect: "text" }}>{n.text}</div>
                  <div className="faint" style={{ fontSize: 10 }}>{new Date(n.createdAt).toLocaleString()}</div>
                </div>
                <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} aria-label="Delete note" onClick={() => api.deleteNote(n.id).then(() => setNotes((x) => x.filter((y) => y.id !== n.id))).catch(error)} />
              </div>
            ))}
          </Panel>
        </div>
      )}
      {del && (
        <Confirm title="Delete recording?" danger confirmLabel="Delete" onCancel={() => setDel(null)} onConfirm={() => { const r = del; setDel(null); api.deleteRecording(r.id).then(loadRecs).catch(error); }}>
          “{del.title}” and its audio file will be permanently deleted. Practice results that used it are kept.
        </Confirm>
      )}
    </div>
  );
}
