import { AudioWaveform, BookOpen, GraduationCap, Home as HomeIcon, Mic, Music2, Plug, Settings as Cog, SlidersHorizontal } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { isTauri } from "./api/client";
import { Button, Notice } from "./components/ui";
import { formatTime } from "./lib/music";
import { useApp, type Workspace } from "./state/app";
import { useEnginePolling } from "./state/polling";
import { Analyzer } from "./workspaces/Analyzer";
import { Coach } from "./workspaces/Coach";
import { FirstRun } from "./workspaces/FirstRun";
import { Home } from "./workspaces/Home";
import { Library } from "./workspaces/Library";
import { Plugins } from "./workspaces/Plugins";
import { SettingsView } from "./workspaces/Settings";
import { SongLab } from "./workspaces/SongLab";
import { Studio } from "./workspaces/Studio";

const NAV: { id: Workspace; label: string; icon: ReactNode; key: string }[] = [
  { id: "home", label: "Home", icon: <HomeIcon size={20} />, key: "1" },
  { id: "analyzer", label: "Analyzer", icon: <AudioWaveform size={20} />, key: "2" },
  { id: "coach", label: "Coach", icon: <GraduationCap size={20} />, key: "3" },
  { id: "songlab", label: "Song Lab", icon: <Music2 size={20} />, key: "4" },
  { id: "studio", label: "Studio", icon: <SlidersHorizontal size={20} />, key: "5" },
  { id: "plugins", label: "Plugins", icon: <Plug size={20} />, key: "6" },
  { id: "library", label: "Progress", icon: <BookOpen size={20} />, key: "7" },
];

function StatusBar() {
  const status = useApp((s) => s.status);
  const capture = useApp((s) => s.capture);
  const info = useApp((s) => s.info);
  const settings = useApp((s) => s.settings);
  const go = useApp((s) => s.go);
  const b = status?.backend ?? info?.engine;
  const recording = status?.recording || capture?.active;
  const problems = (status?.inputUnderruns ?? 0) + (status?.streamUnderruns ?? 0) + (status?.deviceXruns ?? 0);
  return (
    <footer className="statusbar" aria-label="Audio status">
      <span className={`statusbar__item ${status?.deviceLost ? "statusbar__item--danger" : ""}`}>
        <span className={`dot ${status?.deviceLost ? "" : b?.kind === "device" ? "dot--ok" : "dot--warn"}`} />
        {status?.deviceLost ? "Audio device disconnected" : b ? (b.kind === "device" ? `${b.outputName ?? "Output"} · ${b.sampleRate / 1000} kHz` : "No audio device (silent)") : "Audio starting…"}
      </span>
      <span className="statusbar__item" title="Microphone">
        <Mic size={12} />
        {settings?.microphoneEnabled ? (b?.inputName ? b.inputName : "No microphone open") : "Microphone off"}
      </span>
      {recording && (
        <span className="statusbar__item" style={{ color: "var(--rec)" }}>
          <span className="dot dot--rec" />
          Recording {capture?.active ? formatTime(capture.seconds) : ""}
        </span>
      )}
      <span className="statusbar__spacer" />
      {problems > 0 && (
        <span className="statusbar__item statusbar__item--warn" title="Audio dropouts since the engine started">
          {problems} dropout{problems === 1 ? "" : "s"}
        </span>
      )}
      {status && (
        <span className="statusbar__item mono" title="Audio processing load">
          DSP {(status.dspLoad * 100).toFixed(0)}%
        </span>
      )}
      {status && (status.inputLatencyMs > 0 || status.outputLatencyMs > 0) && (
        <span className="statusbar__item mono" title="Latency reported by the audio driver">
          in {status.inputLatencyMs.toFixed(0)} / out {status.outputLatencyMs.toFixed(0)} ms
        </span>
      )}
      <button className="btn btn--ghost btn--sm" onClick={() => go("settings")}>
        Settings
      </button>
    </footer>
  );
}

function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismiss);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.kind}`} role={t.kind === "error" ? "alert" : "status"}>
          <div className="grow">{t.text}</div>
          <button className="btn btn--ghost btn--sm" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
            ×
          </button>
        </div>
      ))}
    </div>
  );
}

export function App() {
  const init = useApp((s) => s.init);
  const workspace = useApp((s) => s.workspace);
  const go = useApp((s) => s.go);
  const settings = useApp((s) => s.settings);
  const loadError = useApp((s) => s.loadError);
  useEnginePolling();

  useEffect(() => {
    void init();
  }, [init]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
        const n = NAV.find((x) => x.key === e.key);
        if (n) {
          e.preventDefault();
          go(n.id);
        } else if (e.key === ",") {
          e.preventDefault();
          go("settings");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go]);

  if (!isTauri()) {
    return (
      <div className="workspace">
        <Notice kind="warn">UTA Vocal Lab is a desktop app. Start it with the installed application (or `npm run tauri dev`).</Notice>
      </div>
    );
  }
  if (loadError) {
    return (
      <div className="workspace">
        <Notice kind="danger">
          UTA Vocal Lab could not start: {loadError}
          <div style={{ marginTop: 8 }}>
            <Button onClick={() => void init()}>Retry</Button>
          </div>
        </Notice>
      </div>
    );
  }
  if (settings && !settings.firstRunDone) return <FirstRun />;

  return (
    <div className="app">
      <nav className="nav" aria-label="Workspaces">
        <img src="/logo.svg" className="nav__logo" alt="UTA Vocal Lab" />
        {NAV.map((n) => (
          <button
            key={n.id}
            className="nav__item"
            aria-current={workspace === n.id ? "page" : undefined}
            title={`${n.label} (Ctrl+${n.key})`}
            onClick={() => go(n.id)}
          >
            {n.icon}
            <span>{n.label}</span>
          </button>
        ))}
        <span className="nav__spacer" />
        <button className="nav__item" aria-current={workspace === "settings" ? "page" : undefined} title="Settings (Ctrl+,)" onClick={() => go("settings")}>
          <Cog size={20} />
          <span>Settings</span>
        </button>
      </nav>
      <main className="main">
        {workspace === "home" && <Home />}
        {workspace === "analyzer" && <Analyzer />}
        {workspace === "coach" && <Coach />}
        {workspace === "songlab" && <SongLab />}
        {workspace === "studio" && <Studio />}
        {workspace === "plugins" && <Plugins />}
        {workspace === "library" && <Library />}
        {workspace === "settings" && <SettingsView />}
      </main>
      <StatusBar />
      <Toasts />
    </div>
  );
}
