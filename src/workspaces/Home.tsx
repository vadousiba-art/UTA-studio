import { ArrowRight, AudioWaveform, FolderOpen, GraduationCap, Mic, Plus, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/client";
import type { CoachOverview, ProjectEntry, RecordingEntry } from "../api/types";
import { Button, Empty, Notice, Panel, SkillLevel, Spinner, Tag } from "../components/ui";
import { formatTime, noteName, relativeDate } from "../lib/music";
import { useApp } from "../state/app";

export function Home() {
  const go = useApp((s) => s.go);
  const openExercise = useApp((s) => s.openExercise);
  const setStudio = useApp((s) => s.setStudio);
  const error = useApp((s) => s.error);
  const settings = useApp((s) => s.settings);
  const [overview, setOverview] = useState<CoachOverview | null>(null);
  const [projects, setProjects] = useState<ProjectEntry[] | null>(null);
  const [recs, setRecs] = useState<RecordingEntry[] | null>(null);

  useEffect(() => {
    api.coachOverview().then(setOverview).catch(error);
    api.recentProjects().then(setProjects).catch(error);
    api.recordings(undefined, 6).then(setRecs).catch(error);
  }, [error]);

  const openProject = async (p: ProjectEntry) => {
    try {
      setStudio(await api.studioOpen(p.path));
      go("studio");
    } catch (e) {
      error(e);
    }
  };

  const rec = overview?.recommendation;
  const started = overview?.skills.filter((s) => s.level > 0) ?? [];
  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Welcome back</h1>
          <p>Short, focused practice beats long, tiring sessions. Pick up where you left off.</p>
        </div>
        <div className="row">
          <Button icon={<AudioWaveform size={16} />} onClick={() => go("analyzer")}>
            Quick analysis
          </Button>
          <Button variant="primary" icon={<GraduationCap size={16} />} onClick={() => go("coach")}>
            Start practice session
          </Button>
        </div>
      </div>

      {!settings?.microphoneEnabled && (
        <div style={{ marginBottom: 16 }}>
          <Notice kind="warn">
            Microphone access is off, so recording is disabled.{" "}
            <Button size="sm" icon={<Mic size={14} />} onClick={() => go("settings")}>
              Open Settings
            </Button>
          </Notice>
        </div>
      )}
      {overview?.health.message && (
        <div style={{ marginBottom: 16 }}>
          <Notice kind={overview.health.recommendProfessional ? "danger" : "warn"}>{overview.health.message}</Notice>
        </div>
      )}

      <div className="grid grid--sidebar">
        <div className="grid">
          <Panel title="Recommended next exercise" variant="accent">
            {!overview ? (
              <Spinner label="Loading your plan…" />
            ) : rec ? (
              <div className="row row--between">
                <div>
                  <div style={{ fontSize: 18, fontWeight: 650 }}>
                    <Sparkles size={16} color="var(--accent)" style={{ verticalAlign: -2, marginRight: 6 }} />
                    {overview.skills.find((s) => s.id === rec.skill)?.title}
                  </div>
                  <div className="muted" style={{ marginTop: 4 }}>
                    {rec.reason}
                  </div>
                  <div className="row" style={{ marginTop: 8 }}>
                    <Tag kind="accent">Level {rec.level}</Tag>
                    <Tag>{overview.session.minutes}-minute session available</Tag>
                  </div>
                </div>
                <Button variant="primary" size="lg" icon={<ArrowRight size={16} />} onClick={() => openExercise(rec.exercise)}>
                  Start
                </Button>
              </div>
            ) : (
              <div className="muted">Nothing is due. Free practice in the Analyzer or Song Lab is a great choice today.</div>
            )}
          </Panel>

          <Panel
            title="Recent projects"
            actions={
              <Button size="sm" icon={<Plus size={14} />} onClick={() => go("studio")}>
                New
              </Button>
            }
          >
            {!projects ? (
              <Spinner />
            ) : projects.length === 0 ? (
              <Empty icon={<FolderOpen size={28} />} title="No projects yet">
                Create a project in the Studio to record multitrack vocals.
              </Empty>
            ) : (
              <div className="list">
                {projects.slice(0, 6).map((p) => (
                  <div key={p.id} className="list__item" role="button" tabIndex={0} onClick={() => void openProject(p)} onKeyDown={(e) => e.key === "Enter" && void openProject(p)}>
                    <FolderOpen size={16} color="var(--text-3)" />
                    <div className="grow">
                      <div>{p.name}</div>
                      <div className="faint" style={{ fontSize: 11 }}>
                        {p.exists ? `opened ${relativeDate(p.lastOpenedAt)}` : "folder not found"}
                      </div>
                    </div>
                    {!p.exists && <Tag kind="warn">missing</Tag>}
                  </div>
                ))}
              </div>
            )}
          </Panel>

          <Panel title="Recent recordings" actions={<Button size="sm" variant="ghost" onClick={() => go("library")}>All</Button>}>
            {!recs ? (
              <Spinner />
            ) : recs.length === 0 ? (
              <Empty icon={<Mic size={28} />} title="No recordings yet">
                Record a phrase in the Analyzer to see your pitch and timing.
              </Empty>
            ) : (
              <table className="table">
                <tbody>
                  {recs.map((r) => (
                    <tr key={r.id}>
                      <td>{r.title}</td>
                      <td className="muted">{r.source}</td>
                      <td className="mono">{formatTime(r.duration)}</td>
                      <td className="faint">{relativeDate(r.createdAt)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Panel>
        </div>

        <div className="grid" style={{ alignContent: "start" }}>
          <Panel title="Skill progress">
            {!overview ? (
              <Spinner />
            ) : started.length === 0 ? (
              <div className="muted">Your skills will appear here after your first lesson.</div>
            ) : (
              <div className="col">
                {started.slice(0, 8).map((s) => (
                  <div key={s.id} className="row row--between">
                    <span>{s.title}</span>
                    <SkillLevel level={s.level} />
                  </div>
                ))}
              </div>
            )}
          </Panel>
          <Panel title="This fortnight">
            <div className="row row--between">
              <span className="muted">Practice days</span>
              <span className="kpi">
                {overview?.practiceDaysLast14 ?? "–"}
                <small>/ 14</small>
              </span>
            </div>
            <div className="row row--between" style={{ marginTop: 8 }}>
              <span className="muted">Reviews due</span>
              <span className="kpi">{overview?.reviews.filter((r) => r.overdue).length ?? "–"}</span>
            </div>
            <div className="row row--between" style={{ marginTop: 8 }}>
              <span className="muted">Comfortable range</span>
              <span className="mono">
                {overview ? `${noteName(overview.voiceRange.low)}–${noteName(overview.voiceRange.high)}` : "–"}
                {overview && !overview.voiceRangeMeasured && <Tag kind="warn">default</Tag>}
              </span>
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}
