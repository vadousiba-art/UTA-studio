import { create } from "zustand";
import { api, ApiError } from "../api/client";
import type { AppInfo, CaptureStatus, EngineStatus, Settings, StudioState } from "../api/types";

export type Workspace = "home" | "analyzer" | "coach" | "songlab" | "studio" | "plugins" | "library" | "settings";

export interface Toast {
  id: number;
  kind: "info" | "ok" | "warn" | "error";
  text: string;
}

interface AppState {
  info: AppInfo | null;
  settings: Settings | null;
  status: EngineStatus | null;
  capture: CaptureStatus | null;
  studio: StudioState | null;
  workspace: Workspace;
  toasts: Toast[];
  loadError: string | null;
  /** Coach deep link: open this exercise when the Coach workspace mounts. */
  pendingExercise: string | null;

  init: () => Promise<void>;
  go: (w: Workspace) => void;
  openExercise: (id: string) => void;
  consumePendingExercise: () => string | null;
  toast: (text: string, kind?: Toast["kind"]) => void;
  dismiss: (id: number) => void;
  error: (e: unknown) => void;
  saveSettings: (patch: Partial<Settings>) => Promise<Settings | null>;
  refreshInfo: () => Promise<void>;
  setStatus: (s: EngineStatus | null) => void;
  setCapture: (c: CaptureStatus | null) => void;
  setStudio: (s: StudioState | null) => void;
}

let toastId = 1;

export const useApp = create<AppState>((set, get) => ({
  info: null,
  settings: null,
  status: null,
  capture: null,
  studio: null,
  workspace: "home",
  toasts: [],
  loadError: null,
  pendingExercise: null,

  init: async () => {
    try {
      const info = await api.appInfo();
      const studio = await api.studioState();
      set({ info, settings: info.settings, studio, loadError: null });
      if (info.engineError) get().toast(info.engineError, "warn");
    } catch (e) {
      set({ loadError: e instanceof Error ? e.message : String(e) });
    }
  },

  go: (w) => set({ workspace: w }),

  openExercise: (id) => set({ pendingExercise: id, workspace: "coach" }),

  consumePendingExercise: () => {
    const p = get().pendingExercise;
    if (p) set({ pendingExercise: null });
    return p;
  },

  toast: (text, kind = "info") => {
    const id = toastId++;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { id, kind, text }] }));
    const ttl = kind === "error" ? 9000 : 4500;
    window.setTimeout(() => get().dismiss(id), ttl);
  },

  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  error: (e) => {
    const msg = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
    get().toast(msg, "error");
  },

  saveSettings: async (patch) => {
    const cur = get().settings;
    if (!cur) return null;
    try {
      const next = await api.updateSettings({ ...cur, ...patch });
      set({ settings: next });
      if ("microphoneEnabled" in patch || "inputDevice" in patch || "outputDevice" in patch || "sampleRate" in patch || "bufferFrames" in patch) {
        await get().refreshInfo();
      }
      return next;
    } catch (e) {
      get().error(e);
      return null;
    }
  },

  refreshInfo: async () => {
    try {
      const info = await api.appInfo();
      set({ info, settings: info.settings });
      if (info.engineError) get().toast(info.engineError, "warn");
    } catch (e) {
      get().error(e);
    }
  },

  setStatus: (status) => set({ status }),
  setCapture: (capture) => set({ capture }),
  setStudio: (studio) => set({ studio }),
}));
