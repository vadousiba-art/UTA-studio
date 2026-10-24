import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../../api/client";
import type { ProjectEntry } from "../../api/types";
import { Button, Empty, Field, Panel, Tag } from "../../components/ui";
import { relativeDate } from "../../lib/music";
import { useApp } from "../../state/app";

export function StudioStart() {
  const setStudio = useApp((s) => s.setStudio);
  const error = useApp((s) => s.error);
  const status = useApp((s) => s.status);
  const [name, setName] = useState("My song");
  const [rate, setRate] = useState<number | "">("");
  const [projects, setProjects] = useState<ProjectEntry[]>([]);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.recentProjects(query || undefined).then(setProjects).catch(error);
  }, [query, error]);

  const create = async () => {
    setBusy(true);
    try {
      setStudio(await api.studioNew(name, undefined, rate === "" ? undefined : rate));
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  const openPath = async (path: string) => {
    setBusy(true);
    try {
      setStudio(await api.studioOpen(path));
    } catch (e) {
      error(e);
    } finally {
      setBusy(false);
    }
  };
  const browse = async () => {
    const p = await open({ directory: true, title: "Open a UTA project folder (.utaproj)" });
    if (typeof p === "string") await openPath(p);
  };

  return (
    <div className="workspace">
      <div className="page-header">
        <div>
          <h1>Recording Studio</h1>
          <p>Multitrack recording, takes and comping, mixing with effects, and WAV export.</p>
        </div>
      </div>
      <div className="grid grid--2">
        <Panel title="New project" variant="accent">
          <div className="stack">
            <Field label="Name">
              <input className="input" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void create()} />
            </Field>
            <Field label="Sample rate" hint={`The audio device currently runs at ${status?.sampleRate ?? "?"} Hz. Projects play only at their own rate.`}>
              <select className="select" value={rate} onChange={(e) => setRate(e.target.value ? Number(e.target.value) : "")}>
                <option value="">Same as the audio device</option>
                <option value={44100}>44.1 kHz</option>
                <option value={48000}>48 kHz</option>
              </select>
            </Field>
            <div className="row">
              <Button variant="primary" icon={<Plus size={16} />} disabled={busy || !name.trim()} onClick={() => void create()}>
                Create project
              </Button>
              <Button icon={<FolderOpen size={16} />} disabled={busy} onClick={() => void browse()}>
                Open folder…
              </Button>
            </div>
          </div>
        </Panel>
        <Panel title="Recent projects">
          <input className="input" type="search" placeholder="Search projects" value={query} onChange={(e) => setQuery(e.target.value)} style={{ width: "100%", marginBottom: 8 }} />
          {projects.length === 0 ? (
            <Empty title={query ? "No matches" : "No projects yet"} />
          ) : (
            <div className="list">
              {projects.map((p) => (
                <div key={p.id} className="list__item" onClick={() => p.exists && void openPath(p.path)}>
                  <FolderOpen size={16} color="var(--text-3)" />
                  <div className="grow">
                    <div>{p.name}</div>
                    <div className="mono faint" style={{ fontSize: 10 }}>{p.path}</div>
                  </div>
                  {p.exists ? <span className="faint" style={{ fontSize: 11 }}>{relativeDate(p.lastOpenedAt)}</span> : <Tag kind="warn">missing</Tag>}
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<Trash2 size={13} />}
                    aria-label={`Remove ${p.name} from the list`}
                    title="Remove from list (does not delete files)"
                    onClick={(e) => {
                      e.stopPropagation();
                      api.forgetProject(p.id).then(() => setProjects((x) => x.filter((y) => y.id !== p.id))).catch(error);
                    }}
                  />
                </div>
              ))}
            </div>
          )}
        </Panel>
      </div>
    </div>
  );
}
