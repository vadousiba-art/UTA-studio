import { ArrowLeft, CheckCircle2, CircleStop, Hand, Lock, Play, RotateCcw, Ruler, Volume2, XCircle } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api/client";
import type { AttemptResult, CoachOverview, ExerciseInfo, ExercisePlan, HealthStatus, PhraseAnalysis, RangeResult } from "../api/types";
import { Button, Empty, Modal, Notice, Panel, ProgressBar, SkillLevel, Spinner, Tabs, Tag } from "../components/ui";
import { noteName, relativeDate } from "../lib/music";
import { useApp } from "../state/app";
import { useLivePitch } from "../state/polling";
import { LiveTuner } from "../viz/LiveTuner";
import { PitchView } from "../viz/PitchView";
import { FeedbackPanel } from "./Analyzer";

const OUTCOME_LABEL: Record<string, string> = {
  success: "Success",
  partial: "Partly there",
  needsWork: "Needs work",
  inconclusive: "Couldn't measure",
  stopped: "Stopped",
};

function outcomeTag(o: string) {
  const kind = o === "success" ? "ok" : o === "partial" ? "accent" : o === "needsWork" ? "warn" : o === "stopped" ? "danger" : undefined;
  return <Tag kind={kind}>{OUTCOME_LABEL[o] ?? o}</Tag>;
}

function DiscomfortButton({ onStopped }: { onStopped: (h: HealthStatus) => void }) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const error = useApp((s) => s.error);
  return (
    <>
      <Button variant="danger" icon={<Hand size={16} />} onClick={() => setOpen(true)} title="Stop immediately if anything hurts">
        I feel pain or discomfort
      </Button>
      {open && (
        <Modal
          title="Stop and rest"
          onClose={() => setOpen(false)}
          actions={
            <Button
              variant="primary"
              onClick={() => {
                api
                  .coachReportDiscomfort(note || undefined)
                  .then((h) => {
                    setOpen(false);
                    onStopped(h);
                  })
                  .catch(error);
              }}
            >
              Stop the exercise
            </Button>
          }
        >
          <div className="stack">
            <div className="muted">
              Thank you for listening to your body. The exercise will stop now and nothing will be graded. Rest your voice, drink some
              water and avoid loud singing or shouting for a while.
            </div>
            <div className="muted">
              If pain, hoarseness or discomfort lasts more than a few days or keeps coming back, please see a qualified voice teacher or a
              medical professional. This app can't assess vocal health.
            </div>
            <textarea className="textarea" placeholder="Optional: what did you feel? (saved only on this computer)" value={note} onChange={(e) => setNote(e.target.value)} />
          </div>
        </Modal>
      )}
    </>
  );
}

function RangeCheck({ onDone }: { onDone: () => void }) {
  const capture = useApp((s) => s.capture);
  const error = useApp((s) => s.error);
  const settings = useApp((s) => s.settings);
  const refreshInfo = useApp((s) => s.refreshInfo);
  const [result, setResult] = useState<RangeResult | null>(null);
  const [busy, setBusy] = useState(false);
  const live = useLivePitch(true);
  const active = capture?.active && (capture.kind === "rangeLow" || capture.kind === "rangeHigh");
  const run = async (high: boolean) => {
    setBusy(true);
    try {
      if (active) {
        const r = await api.rangeStop();
        setResult(r);
        await refreshInfo();
      } else {
        setResult(null);
        await api.rangeStart(high);
      }
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Panel title="Comfortable range check" actions={<Button size="sm" variant="ghost" onClick={onDone}>Close</Button>}>
      <div className="stack">
        <div className="muted">
          Sing your <strong>lowest comfortable</strong> note on “ah” and hold it for about two seconds — not the lowest note you can
          possibly reach. Then do the same for your <strong>highest comfortable</strong> note. Exercises will stay inside this range.
        </div>
        <LiveTuner framesRef={live} />
        <div className="row">
          {active ? (
            <Button variant="primary" icon={<CircleStop size={16} />} disabled={busy} onClick={() => void run(false)}>
              Stop and measure
            </Button>
          ) : (
            <>
              <Button disabled={busy || !settings?.microphoneEnabled} onClick={() => void run(false)}>Record lowest comfortable note</Button>
              <Button disabled={busy || !settings?.microphoneEnabled} onClick={() => void run(true)}>Record highest comfortable note</Button>
            </>
          )}
          {busy && <Spinner />}
        </div>
        {result && <Notice kind={result.midi != null ? "ok" : "warn"}>{result.message}</Notice>}
        <div className="faint">
          Current range: {settings ? `${noteName(settings.voiceRange.low)} – ${noteName(settings.voiceRange.high)}` : "—"}
        </div>
      </div>
    </Panel>
  );
}

type Phase = "intro" | "recording" | "analysing" | "result" | "stopped";

function Lesson({ exerciseId, onExit }: { exerciseId: string; onExit: () => void }) {
  const error = useApp((s) => s.error);
  const settings = useApp((s) => s.settings);
  const [plan, setPlan] = useState<ExercisePlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>("intro");
  const [elapsed, setElapsed] = useState(0);
  const [result, setResult] = useState<AttemptResult | null>(null);
  const [analysis, setAnalysis] = useState<PhraseAnalysis | null>(null);
  const [health, setHealth] = useState<HealthStatus | null>(null);
  const [rated, setRated] = useState<number | null>(null);
  const timer = useRef<number | null>(null);
  const live = useLivePitch(phase === "intro" || phase === "recording");

  const load = useCallback(() => {
    setPlanError(null);
    api
      .coachPlan(exerciseId)
      .then(setPlan)
      .catch((e) => setPlanError(e.message));
  }, [exerciseId]);
  useEffect(load, [load]);
  useEffect(() => () => {
    if (timer.current) window.clearInterval(timer.current);
  }, []);

  const stop = useCallback(async () => {
    if (timer.current) window.clearInterval(timer.current);
    timer.current = null;
    setPhase("analysing");
    try {
      const r = await api.coachStopAttempt();
      setResult(r);
      setRated(null);
      setPhase("result");
      api.recordingAnalysis(r.recording.id).then((x) => setAnalysis(x.analysis)).catch(() => setAnalysis(null));
    } catch (e) {
      error(e);
      setPhase("intro");
    }
  }, [error]);

  const start = async () => {
    if (!plan) return;
    try {
      await api.stopReference().catch(() => undefined);
      await api.coachStartAttempt(plan);
      setPhase("recording");
      setResult(null);
      setAnalysis(null);
      const t0 = performance.now();
      timer.current = window.setInterval(() => {
        const s = (performance.now() - t0) / 1000;
        setElapsed(s);
        if (s >= plan.recordSeconds) void stop();
      }, 50);
    } catch (e) {
      error(e);
    }
  };

  const retry = () => {
    setPhase("intro");
    setResult(null);
    setAnalysis(null);
    load();
  };

  if (planError) {
    return (
      <div className="workspace">
        <Button variant="ghost" icon={<ArrowLeft size={16} />} onClick={onExit}>Back</Button>
        <div style={{ marginTop: 12 }}>
          <Notice kind="warn">{planError}</Notice>
        </div>
      </div>
    );
  }
  if (!plan) return <div className="workspace"><Spinner label="Preparing exercise…" /></div>;

  const beat = plan.clickBpm ? 60 / plan.clickBpm : 0.75;
  const countIn = plan.targets[0]?.start ?? plan.beats[0] ?? 4 * beat;
  const countdown = phase === "recording" && elapsed < countIn ? Math.ceil((countIn - elapsed) / beat) : null;
  const activeTarget = phase === "recording" ? plan.targets.find((t) => elapsed >= t.start && elapsed < t.start + t.duration) : undefined;
  const activeSyllable = (() => {
    if (phase !== "recording") return null;
    if (plan.beats.length) {
      const i = plan.beats.findIndex((b, k) => elapsed >= b && elapsed < (plan.beats[k + 1] ?? b + beat));
      return i >= 0 ? plan.syllables[i] ?? null : null;
    }
    const i = plan.targets.findIndex((t) => elapsed >= t.start && elapsed < t.start + t.duration);
    return i >= 0 ? plan.syllables[i] ?? null : null;
  })();

  return (
    <div className="workspace">
      <div className="row row--between" style={{ marginBottom: 16 }}>
        <Button variant="ghost" icon={<ArrowLeft size={16} />} disabled={phase === "recording" || phase === "analysing"} onClick={onExit}>
          All lessons
        </Button>
        <DiscomfortButton
          onStopped={(h) => {
            if (timer.current) window.clearInterval(timer.current);
            timer.current = null;
            setHealth(h);
            setPhase("stopped");
          }}
        />
      </div>
      <div className="grid grid--sidebar">
        <div className="grid" style={{ alignContent: "start" }}>
          <Panel variant="accent">
            <div className="row row--between">
              <div>
                <div className="faint" style={{ fontSize: 11, textTransform: "uppercase", letterSpacing: ".08em" }}>Objective</div>
                <h2 style={{ margin: "4px 0", fontSize: 22 }}>{plan.title}</h2>
                <div className="muted">{plan.objective}</div>
              </div>
              <div className="col" style={{ alignItems: "flex-end" }}>
                <Tag kind="accent">Level {plan.level}</Tag>
                {plan.noteLabels.length > 0 && <span className="mono faint">{plan.noteLabels.slice(0, 6).join(" · ")}</span>}
              </div>
            </div>
          </Panel>

          {phase === "stopped" ? (
            <Panel>
              <div className="stack">
                <Notice kind="danger">{health?.message ?? "The exercise was stopped. Rest your voice."}</Notice>
                <div className="muted">Nothing was graded. Demanding exercises are paused for a couple of days; gentle ones stay available.</div>
                <Button onClick={onExit}>Back to lessons</Button>
              </div>
            </Panel>
          ) : (
            <Panel title="Steps">
              <ol style={{ margin: 0, paddingLeft: 20 }} className="stack">
                {plan.steps.map((s) => (
                  <li key={s}>{s}</li>
                ))}
              </ol>
              {plan.safety && <div className="faint" style={{ marginTop: 12, fontSize: 12 }}>{plan.safety}</div>}
              <div className="row" style={{ marginTop: 16 }}>
                <Button icon={<Volume2 size={16} />} disabled={phase === "recording" || plan.demo.length === 0} onClick={() => api.coachDemo(plan).catch(error)}>
                  Hear the demo
                </Button>
                {phase === "recording" ? (
                  <Button variant="primary" icon={<CircleStop size={16} />} onClick={() => void stop()}>
                    Finish early
                  </Button>
                ) : (
                  <Button variant="rec" size="lg" disabled={phase === "analysing" || !settings?.microphoneEnabled} onClick={() => void start()}>
                    {phase === "result" ? "Record again" : "Record attempt"}
                  </Button>
                )}
                {phase === "analysing" && <Spinner label="Measuring…" />}
              </div>
              {phase === "recording" && (
                <div style={{ marginTop: 16 }} className="stack">
                  <ProgressBar value={elapsed / plan.recordSeconds} label="Recording progress" />
                  <div className="row" style={{ gap: 20 }}>
                    <div style={{ fontSize: 42, fontWeight: 700, minWidth: 90, color: countdown ? "var(--warn)" : "var(--rec)" }}>
                      {countdown ?? "●"}
                    </div>
                    <div>
                      <div className="faint">{countdown ? "Count-in…" : "Sing now"}</div>
                      <div style={{ fontSize: 26 }} className="jp">
                        {activeSyllable ?? (activeTarget ? noteName(activeTarget.midi) : "")}
                      </div>
                    </div>
                  </div>
                </div>
              )}
              {(phase === "intro" || phase === "recording") && (
                <div style={{ marginTop: 16 }}>
                  <LiveTuner framesRef={live} target={activeTarget?.midi ?? (phase === "intro" ? plan.targets[0]?.midi : null)} />
                </div>
              )}
            </Panel>
          )}

          {phase === "result" && result && (
            <Panel
              title="Your attempt"
              actions={
                <>
                  {outcomeTag(result.evaluation.outcome)}
                  <Button size="sm" icon={<Play size={14} />} onClick={() => api.previewFile(result.recording.path).catch(error)}>
                    Listen
                  </Button>
                </>
              }
            >
              {analysis && (
                <PitchView
                  frames={analysis.frames}
                  duration={analysis.duration}
                  targets={plan.targets}
                  toleranceCents={plan.criteria.pitchToleranceCents || 25}
                  height={240}
                />
              )}
              {result.evaluation.checks.length > 0 && (
                <div style={{ marginTop: 12 }}>
                  {result.evaluation.checks.map((c) => (
                    <div key={c.key} className="check-row">
                      {c.pass ? <CheckCircle2 size={16} color="var(--ok)" /> : <XCircle size={16} color="var(--warn)" />}
                      <span className="grow">{c.label}</span>
                      <span className="mono muted">{c.value}</span>
                    </div>
                  ))}
                </div>
              )}
              <div className="row row--between" style={{ marginTop: 12 }}>
                <div className="muted">{result.decisionText}</div>
                {result.nextReview && <Tag>review {new Date(result.nextReview).toLocaleDateString()}</Tag>}
              </div>
              <div className="row" style={{ marginTop: 12 }}>
                <span className="muted">How hard did that feel?</span>
                {[1, 2, 3, 4, 5].map((d) => (
                  <Button
                    key={d}
                    size="sm"
                    aria-pressed={rated === d}
                    onClick={() => {
                      setRated(d);
                      api.coachRateAttempt(result.attempt.id, d).catch(error);
                    }}
                  >
                    {["Easy", "OK", "Medium", "Hard", "Too hard"][d - 1]}
                  </Button>
                ))}
                <span className="grow" />
                <Button icon={<RotateCcw size={14} />} onClick={retry}>
                  Retry
                </Button>
              </div>
            </Panel>
          )}
        </div>
        <div className="grid" style={{ alignContent: "start" }}>
          {phase === "result" && result && <FeedbackPanel fb={result.evaluation.feedback} title="Coach feedback" />}
          <Panel title="How this works">
            <ol style={{ margin: 0, paddingLeft: 18 }} className="muted">
              <li>Read the objective and hear the demo.</li>
              <li>Record your attempt after the count-in.</li>
              <li>The app measures pitch, timing and steadiness from your recording.</li>
              <li>You get one main thing to work on and a goal for the retry.</li>
              <li>Results are saved and a review is scheduled.</li>
            </ol>
          </Panel>
        </div>
      </div>
    </div>
  );
}

export function Coach() {
  const error = useApp((s) => s.error);
  const consume = useApp((s) => s.consumePendingExercise);
  const [overview, setOverview] = useState<CoachOverview | null>(null);
  const [lesson, setLesson] = useState<string | null>(() => consume());
  const [tab, setTab] = useState<"plan" | "skills" | "library">("plan");
  const [library, setLibrary] = useState<ExerciseInfo[]>([]);
  const [range, setRange] = useState(false);

  const load = useCallback(() => {
    api.coachOverview().then(setOverview).catch(error);
  }, [error]);
  useEffect(() => {
    if (!lesson) load();
  }, [lesson, load]);
  useEffect(() => {
    api.exerciseLibrary().then(setLibrary).catch(error);
  }, [error]);

  if (lesson) return <Lesson exerciseId={lesson} onExit={() => setLesson(null)} />;
  if (!overview) return <div className="workspace"><Spinner label="Loading your training plan…" /></div>;

  const titleOf = (id: string) => library.find((e) => e.id === id)?.title ?? id;
  const stages = Array.from(new Set(overview.skills.map((s) => s.stage))).sort();
  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Vocal Coach</h1>
          <p>The UTA Vocal Transformation Method — one skill at a time, adapted to what you actually demonstrate.</p>
        </div>
        <Button icon={<Ruler size={16} />} onClick={() => setRange(true)}>
          Range check
        </Button>
      </div>
      {overview.health.message && (
        <div style={{ marginBottom: 12 }}>
          <Notice kind={overview.health.recommendProfessional ? "danger" : "warn"}>{overview.health.message}</Notice>
        </div>
      )}
      {!overview.voiceRangeMeasured && !range && (
        <div style={{ marginBottom: 12 }}>
          <Notice kind="info">
            Exercises currently use a default range ({noteName(overview.voiceRange.low)}–{noteName(overview.voiceRange.high)}).{" "}
            <Button size="sm" onClick={() => setRange(true)}>Measure your comfortable range</Button>
          </Notice>
        </div>
      )}
      {range && (
        <div style={{ marginBottom: 16 }}>
          <RangeCheck onDone={() => { setRange(false); load(); }} />
        </div>
      )}
      <Tabs
        tabs={[
          { id: "plan", label: "Today" },
          { id: "skills", label: "Skill map" },
          { id: "library", label: "Exercise library" },
        ]}
        value={tab}
        onChange={setTab}
      />
      {tab === "plan" && (
        <div className="grid grid--sidebar">
          <div className="grid" style={{ alignContent: "start" }}>
            <Panel title={`${overview.session.minutes}-minute session`} variant="accent">
              <div className="list">
                {overview.session.items.map((it, i) => (
                  <div key={it.exercise} className="list__item" onClick={() => setLesson(it.exercise)} role="button" tabIndex={0} onKeyDown={(e) => e.key === "Enter" && setLesson(it.exercise)}>
                    <span className="mono faint">{i + 1}</span>
                    <div className="grow">
                      <div>{titleOf(it.exercise)}</div>
                      <div className="faint" style={{ fontSize: 11 }}>{it.reason}</div>
                    </div>
                    <Tag>L{it.level}</Tag>
                  </div>
                ))}
              </div>
            </Panel>
            <Panel title="Recent attempts">
              {overview.recent.length === 0 ? (
                <Empty title="No attempts yet">Start with the first exercise in today's session.</Empty>
              ) : (
                <table className="table">
                  <tbody>
                    {overview.recent.slice(0, 10).map((a) => (
                      <tr key={a.id}>
                        <td>{titleOf(a.exerciseId)}</td>
                        <td>L{a.level}</td>
                        <td>{outcomeTag(a.outcome)}</td>
                        <td className="faint">{relativeDate(a.createdAt)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </Panel>
          </div>
          <div className="grid" style={{ alignContent: "start" }}>
            <Panel title="Reviews">
              {overview.reviews.length === 0 ? (
                <div className="muted">Exercises you practise come back for short reviews on a spaced schedule.</div>
              ) : (
                <div className="list">
                  {overview.reviews.slice(0, 8).map((r) => (
                    <div key={r.exerciseId} className="list__item" onClick={() => setLesson(r.exerciseId)}>
                      <div className="grow">{r.exerciseTitle}</div>
                      {r.overdue ? <Tag kind="warn">due</Tag> : <span className="faint" style={{ fontSize: 11 }}>{new Date(r.due).toLocaleDateString()}</span>}
                    </div>
                  ))}
                </div>
              )}
            </Panel>
            <Panel title="About the coach">
              <div className="muted" style={{ fontSize: 12 }}>{overview.disclaimer}</div>
            </Panel>
          </div>
        </div>
      )}
      {tab === "skills" &&
        stages.map((st) => (
          <div key={st} style={{ marginBottom: 20 }}>
            <h3 style={{ margin: "0 0 10px", color: "var(--text-2)" }}>
              Stage {st} · {overview.skills.find((s) => s.stage === st)?.stageTitle}
            </h3>
            <div className="grid grid--3">
              {overview.skills
                .filter((s) => s.stage === st)
                .map((s) => (
                  <Panel key={s.id} variant={s.unlocked ? undefined : "flat"}>
                    <div className="row row--between">
                      <strong>{s.title}</strong>
                      {s.unlocked ? <SkillLevel level={s.level} /> : <Lock size={14} color="var(--text-3)" />}
                    </div>
                    <div className="muted" style={{ fontSize: 12, margin: "6px 0 10px" }}>{s.summary}</div>
                    {s.unlocked ? (
                      s.exercises.length > 0 ? (
                        <div className="row row--wrap">
                          {s.exercises.map((e) => (
                            <Button key={e} size="sm" onClick={() => setLesson(e)}>
                              {titleOf(e)}
                            </Button>
                          ))}
                        </div>
                      ) : (
                        <div className="faint" style={{ fontSize: 12 }}>Practised in the Song Lab and Studio.</div>
                      )
                    ) : (
                      <div className="faint" style={{ fontSize: 12 }}>Unlocks after: {s.requires.join(", ")}</div>
                    )}
                  </Panel>
                ))}
            </div>
          </div>
        ))}
      {tab === "library" && (
        <div className="grid grid--3">
          {library.map((e) => {
            const s = overview.skills.find((x) => x.id === e.skill);
            const open = e.kind === "warmup" || s?.unlocked;
            return (
              <Panel key={e.id} variant={open ? undefined : "flat"}>
                <div className="row row--between">
                  <strong>{e.title}</strong>
                  {e.kind === "warmup" ? <Tag>warm-up</Tag> : <Tag>{s?.title}</Tag>}
                </div>
                <div className="muted" style={{ fontSize: 12, margin: "6px 0 10px" }}>{e.objective}</div>
                <Button size="sm" disabled={!open} icon={open ? undefined : <Lock size={12} />} onClick={() => setLesson(e.id)}>
                  {open ? "Open" : "Locked"}
                </Button>
              </Panel>
            );
          })}
        </div>
      )}
    </div>
  );
}
