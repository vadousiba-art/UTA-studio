import { Mic, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { Button, Notice, Panel } from "../components/ui";
import { useApp } from "../state/app";

/** First launch: explain privacy and ask for microphone access explicitly. */
export function FirstRun() {
  const info = useApp((s) => s.info);
  const saveSettings = useApp((s) => s.saveSettings);
  const [busy, setBusy] = useState(false);

  const finish = async (mic: boolean) => {
    setBusy(true);
    await saveSettings({ microphoneEnabled: mic, firstRunDone: true });
    setBusy(false);
  };

  return (
    <div className="workspace" style={{ display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ maxWidth: 640 }} className="stack">
        <div className="row" style={{ gap: 14 }}>
          <img src="/logo.svg" width={56} height={56} alt="" />
          <div>
            <h1 style={{ margin: 0, fontSize: 28 }}>UTA Vocal Lab</h1>
            <div className="muted">Singing coach and vocal recording studio</div>
          </div>
        </div>
        <Panel title="Before you start">
          <div className="stack">
            <div className="row" style={{ alignItems: "flex-start" }}>
              <ShieldCheck size={18} color="var(--ok)" />
              <div>
                <strong>Everything stays on this computer.</strong>
                <div className="muted">
                  No account, no cloud, no uploads. Recordings, projects and practice history are saved in your own folders:
                  <div className="mono faint" style={{ fontSize: 11, marginTop: 4 }}>{info?.paths.dataDir}</div>
                </div>
              </div>
            </div>
            <div className="row" style={{ alignItems: "flex-start" }}>
              <Mic size={18} color="var(--accent)" />
              <div>
                <strong>Microphone access</strong>
                <div className="muted">
                  The app only opens your microphone after you allow it here, and the status bar always shows which input is
                  open and when recording is running. You can turn it off any time in Settings.
                </div>
              </div>
            </div>
            <Notice kind="info">{info?.disclaimer}</Notice>
          </div>
        </Panel>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <Button disabled={busy} onClick={() => void finish(false)}>
            Not now
          </Button>
          <Button variant="primary" size="lg" disabled={busy} icon={<Mic size={16} />} onClick={() => void finish(true)}>
            Allow microphone and start
          </Button>
        </div>
      </div>
    </div>
  );
}
