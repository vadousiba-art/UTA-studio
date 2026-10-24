// Thin, typed wrapper over Tauri IPC. All errors become `ApiError`.
import { invoke } from "@tauri-apps/api/core";
import type * as T from "./types";

export class ApiError extends Error {
  kind: string;
  constructor(kind: string, message: string) {
    super(message);
    this.kind = kind;
    this.name = "ApiError";
  }
}

function normalize(e: unknown): ApiError {
  if (e instanceof ApiError) return e;
  if (e && typeof e === "object" && "message" in e) {
    const p = e as { kind?: string; message: string };
    return new ApiError(p.kind ?? "other", String(p.message));
  }
  return new ApiError("other", String(e));
}

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<R>(cmd: string, args?: Record<string, unknown>): Promise<R> {
  if (!isTauri()) {
    throw new ApiError("noBackend", "UTA Vocal Lab must run inside the desktop app.");
  }
  try {
    return await invoke<R>(cmd, args);
  } catch (e) {
    throw normalize(e);
  }
}

export const api = {
  // app
  appInfo: () => call<T.AppInfo>("app_info"),
  getSettings: () => call<T.Settings>("get_settings"),
  updateSettings: (settings: T.Settings) => call<T.Settings>("update_settings", { settings }),
  listDevices: () => call<T.DeviceList>("list_devices"),
  restartEngine: () => call<T.BackendInfo>("restart_engine"),
  engineStatus: () => call<T.EngineStatus>("engine_status"),
  resetClips: () => call<void>("reset_clips"),
  setLivePitch: (on: boolean) => call<void>("set_live_pitch", { on }),
  livePitch: (since: number) => call<T.LiveFrame[]>("live_pitch", { since }),
  playReference: (midi: number, seconds: number) => call<void>("play_reference", { midi, seconds }),
  stopReference: () => call<void>("stop_reference"),
  previewFile: (path: string, startSeconds = 0) => call<number>("preview_file", { path, startSeconds }),
  stopPreview: () => call<void>("stop_preview"),
  deleteTrainingHistory: () => call<void>("delete_training_history"),
  backupLibrary: () => call<string>("backup_library"),

  // analyzer
  analyzerStart: () => call<void>("analyzer_start"),
  analyzerStop: () => call<T.AnalyzerResult>("analyzer_stop"),
  captureStatus: () => call<T.CaptureStatus>("capture_status"),
  captureCancel: () => call<void>("capture_cancel"),
  recordings: (source?: string, limit?: number) => call<T.RecordingEntry[]>("recordings", { source, limit }),
  recordingAnalysis: (id: string) => call<T.AnalyzerResult>("recording_analysis", { id }),
  renameRecording: (id: string, title: string) => call<void>("rename_recording", { id, title }),
  deleteRecording: (id: string) => call<void>("delete_recording", { id }),
  compareRecordings: (a: string, b: string) => call<T.RecordingComparison>("compare_recordings", { a, b }),
  waveform: (path: string, start: number, end: number | null, buckets: number) =>
    call<number[]>("waveform", { path, start, end, buckets }),

  // coach
  coachOverview: () => call<T.CoachOverview>("coach_overview"),
  coachPlan: (exerciseId: string, level?: number) => call<T.ExercisePlan>("coach_plan", { exerciseId, level }),
  coachDemo: (plan: T.ExercisePlan) => call<void>("coach_demo", { plan }),
  coachStartAttempt: (plan: T.ExercisePlan) => call<void>("coach_start_attempt", { plan }),
  coachStopAttempt: (userDifficulty?: number, notes?: string) =>
    call<T.AttemptResult>("coach_stop_attempt", { userDifficulty, notes }),
  coachReportDiscomfort: (note?: string) => call<T.HealthStatus>("coach_report_discomfort", { note }),
  coachRateAttempt: (id: string, difficulty: number) => call<void>("coach_rate_attempt", { id, difficulty }),
  rangeStart: (high: boolean) => call<void>("range_start", { high }),
  rangeStop: () => call<T.RangeResult>("range_stop"),
  skillHistory: (skill: string) => call<T.SkillHistoryPoint[]>("skill_history", { skill }),
  exerciseLibrary: () => call<T.ExerciseInfo[]>("exercise_library"),
  addNote: (text: string, skill?: string) => call<T.NoteEntry>("add_note", { text, skill }),
  notes: () => call<T.NoteEntry[]>("notes"),
  deleteNote: (id: string) => call<void>("delete_note", { id }),

  // studio
  studioNew: (name: string, folder?: string, sampleRate?: number) =>
    call<T.StudioState>("studio_new", { name, folder, sampleRate }),
  studioOpen: (path: string) => call<T.StudioState>("studio_open", { path }),
  studioClose: (save: boolean) => call<void>("studio_close", { save }),
  studioSave: () => call<T.StudioState>("studio_save"),
  studioState: () => call<T.StudioState | null>("studio_state"),
  studioEdit: (op: T.EditOp, transient = false) => call<T.StudioState | null>("studio_edit", { op, transient }),
  studioUndo: () => call<T.StudioState>("studio_undo"),
  studioRedo: () => call<T.StudioState>("studio_redo"),
  studioPlay: (from?: number) => call<void>("studio_play", { from }),
  studioStop: () => call<void>("studio_stop"),
  studioLocate: (position: number) => call<void>("studio_locate", { position }),
  studioRecord: (from?: number) => call<void>("studio_record", { from }),
  studioStopRecording: () => call<T.RecordStopResult>("studio_stop_recording"),
  studioImport: (files: string[], track: string | null, position: number) =>
    call<number>("studio_import", { files, track, position }),
  studioExport: (path: string, options: T.RenderOptions) => call<number>("studio_export", { path, options }),
  studioAssetWaveform: (asset: string, start: number, end: number | null, buckets: number) =>
    call<number[]>("studio_asset_waveform", { asset, start, end, buckets }),
  studioRevisions: () => call<T.RevisionInfo[]>("studio_revisions"),
  studioLoadRevision: (id: number) => call<T.StudioState>("studio_load_revision", { id }),
  studioBackup: () => call<string>("studio_backup"),
  studioRelinkAsset: (asset: string, file: string) => call<T.StudioState>("studio_relink_asset", { asset, file }),
  recentProjects: (query?: string) => call<T.ProjectEntry[]>("recent_projects", { query }),
  forgetProject: (id: string) => call<void>("forget_project", { id }),

  // effects
  effectCatalog: () => call<T.EffectCatalogEntry[]>("effect_catalog"),
  presets: (effectKind?: string) => call<T.PresetEntry[]>("presets", { effectKind }),
  savePreset: (effectKind: string, name: string, params: Record<string, number>) =>
    call<T.PresetEntry>("save_preset", { effectKind, name, params }),
  deletePreset: (id: string) => call<void>("delete_preset", { id }),

  // song lab
  songs: () => call<T.SongSummary[]>("songs"),
  songCreate: (title: string, artist: string) => call<T.SongView>("song_create", { title, artist }),
  songGet: (id: string) => call<T.SongView>("song_get", { id }),
  songUpdate: (id: string, update: T.SongUpdate) => call<T.SongView>("song_update", { id, update }),
  songImportLyrics: (id: string, text: string) => call<T.SongView>("song_import_lyrics", { id, text }),
  songImportLyricsFile: (id: string, path: string) => call<T.SongView>("song_import_lyrics_file", { id, path }),
  songDelete: (id: string) => call<void>("song_delete", { id }),
  songImportAudio: (id: string, track: T.SongTrack, path: string) => call<number>("song_import_audio", { id, track, path }),
  songRemoveAudio: (id: string, track: T.SongTrack) => call<T.SongView>("song_remove_audio", { id, track }),
  songPrepare: (id: string, speed: number) => call<number>("song_prepare", { id, speed }),
  songPlay: (id: string, track: T.SongTrack, from: number, to: number | null, looped: boolean, speed: number) =>
    call<T.SongPlayback>("song_play", { id, track, from, to, looped, speed }),
  songStop: () => call<void>("song_stop"),
  songAttemptStart: (id: string, phrase: string, speed: number, withBacking: boolean) =>
    call<T.SongPlayback>("song_attempt_start", { id, phrase, speed, withBacking }),
  songAttemptStop: () => call<T.SongAttemptResult>("song_attempt_stop"),
  songReferenceContour: (id: string, phrase: string) => call<T.PitchFrame[]>("song_reference_contour", { id, phrase }),

  // plugins
  pluginCatalog: () => call<T.PluginCatalog>("plugin_catalog"),
  pluginScan: (force: boolean) => call<number>("plugin_scan", { force }),
  pluginSetEnabled: (path: string, enabled: boolean) => call<void>("plugin_set_enabled", { path, enabled }),
  pluginForget: (path: string) => call<void>("plugin_forget", { path }),
  studioPluginParams: (track: string | null, slot: string) => call<T.PluginParams>("studio_plugin_params", { track, slot }),
  studioReloadPlugin: (track: string | null, slot: string) => call<T.StudioState>("studio_reload_plugin", { track, slot }),

  // jobs
  jobs: () => call<T.JobInfo[]>("jobs_list"),
  cancelJob: (id: number) => call<boolean>("job_cancel", { id }),
  clearJobs: () => call<void>("jobs_clear"),
};

export type Api = typeof api;
