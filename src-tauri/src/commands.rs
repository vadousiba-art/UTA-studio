//! IPC commands. Names map 1:1 to `src/api/*.ts` on the frontend.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;
use uta_audio::device::{BackendInfo, DeviceList};
use uta_audio::render::RenderOptions;
use uta_audio::EngineStatus;
use uta_coach::exercises::ExercisePlan;
use uta_coach::progression::HealthStatus;
use uta_core::analyzer::{AnalyzerResult, CaptureStatus, RecordingComparison};
use uta_core::coach::{AttemptResult, CoachOverview, RangeResult, SkillHistoryPoint};
use uta_core::jobs::JobInfo;
use uta_core::live::LiveFrame;
use uta_core::studio::{EditOp, RecordStopResult, StudioState};
use uta_core::{AppCore, AppInfo, ErrorPayload, Settings};
use uta_project::library::{NoteEntry, PresetEntry, ProjectEntry, RecordingEntry};
use uta_project::{Id, RevisionInfo};

type Core<'a> = State<'a, Arc<AppCore>>;
type R<T> = Result<T, ErrorPayload>;

async fn run<T: Send + 'static>(core: &Arc<AppCore>, f: impl FnOnce(&Arc<AppCore>) -> uta_core::Result<T> + Send + 'static) -> R<T> {
    let c = core.clone();
    tauri::async_runtime::spawn_blocking(move || f(&c))
        .await
        .map_err(|e| ErrorPayload { kind: "internal".into(), message: format!("internal error: {e}") })?
        .map_err(|e| ErrorPayload::from(&e))
}

// ------------------------------------------------------------------ app

#[tauri::command]
async fn app_info(core: Core<'_>) -> R<AppInfo> {
    run(&core, |c| Ok(c.info())).await
}

#[tauri::command]
async fn get_settings(core: Core<'_>) -> R<Settings> {
    run(&core, |c| Ok(c.settings())).await
}

#[tauri::command]
async fn update_settings(core: Core<'_>, settings: Settings) -> R<Settings> {
    run(&core, move |c| c.update_settings(settings)).await
}

#[tauri::command]
async fn list_devices(core: Core<'_>) -> R<DeviceList> {
    run(&core, |c| c.list_devices()).await
}

#[tauri::command]
async fn restart_engine(core: Core<'_>) -> R<BackendInfo> {
    run(&core, |c| c.restart_engine()).await
}

#[tauri::command]
async fn engine_status(core: Core<'_>) -> R<EngineStatus> {
    run(&core, |c| c.engine_status()).await
}

#[tauri::command]
async fn reset_clips(core: Core<'_>) -> R<()> {
    run(&core, |c| c.reset_clip_indicators()).await
}

#[tauri::command]
async fn set_live_pitch(core: Core<'_>, on: bool) -> R<()> {
    run(&core, move |c| {
        c.set_live_pitch(on);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn live_pitch(core: Core<'_>, since: u64) -> R<Vec<LiveFrame>> {
    run(&core, move |c| Ok(c.live_pitch(since))).await
}

#[tauri::command]
async fn play_reference(core: Core<'_>, midi: f32, seconds: f32) -> R<()> {
    run(&core, move |c| c.play_reference(midi, seconds)).await
}

#[tauri::command]
async fn stop_reference(core: Core<'_>) -> R<()> {
    run(&core, |c| c.stop_reference()).await
}

#[tauri::command]
async fn preview_file(core: Core<'_>, path: String, start_seconds: f32) -> R<u64> {
    run(&core, move |c| c.preview_file(PathBuf::from(path), start_seconds)).await
}

#[tauri::command]
async fn stop_preview(core: Core<'_>) -> R<()> {
    run(&core, |c| c.stop_preview()).await
}

#[tauri::command]
async fn delete_training_history(core: Core<'_>) -> R<()> {
    run(&core, |c| c.delete_training_history()).await
}

#[tauri::command]
async fn backup_library(core: Core<'_>) -> R<String> {
    run(&core, |c| Ok(c.backup_library()?.to_string_lossy().to_string())).await
}

// ------------------------------------------------------------- analyzer

#[tauri::command]
async fn analyzer_start(core: Core<'_>) -> R<()> {
    run(&core, |c| c.analyzer_start()).await
}

#[tauri::command]
async fn analyzer_stop(core: Core<'_>) -> R<AnalyzerResult> {
    run(&core, |c| c.analyzer_stop()).await
}

#[tauri::command]
async fn capture_status(core: Core<'_>) -> R<CaptureStatus> {
    run(&core, |c| Ok(c.capture_status())).await
}

#[tauri::command]
async fn capture_cancel(core: Core<'_>) -> R<()> {
    run(&core, |c| c.capture_cancel()).await
}

#[tauri::command]
async fn recordings(core: Core<'_>, source: Option<String>, limit: Option<usize>) -> R<Vec<RecordingEntry>> {
    run(&core, move |c| c.recordings(source.as_deref(), limit.unwrap_or(100))).await
}

#[tauri::command]
async fn recording_analysis(core: Core<'_>, id: String) -> R<AnalyzerResult> {
    run(&core, move |c| c.recording_analysis(&id)).await
}

#[tauri::command]
async fn rename_recording(core: Core<'_>, id: String, title: String) -> R<()> {
    run(&core, move |c| c.rename_recording(&id, &title)).await
}

#[tauri::command]
async fn delete_recording(core: Core<'_>, id: String) -> R<()> {
    run(&core, move |c| c.delete_recording(&id)).await
}

#[tauri::command]
async fn compare_recordings(core: Core<'_>, a: String, b: String) -> R<RecordingComparison> {
    run(&core, move |c| c.compare_recordings(&a, &b)).await
}

#[tauri::command]
async fn waveform(core: Core<'_>, path: String, start: u64, end: Option<u64>, buckets: usize) -> R<Vec<f32>> {
    run(&core, move |c| c.waveform(&PathBuf::from(path), start, end, buckets)).await
}

// ---------------------------------------------------------------- coach

#[tauri::command]
async fn coach_overview(core: Core<'_>) -> R<CoachOverview> {
    run(&core, |c| c.coach_overview()).await
}

#[tauri::command]
async fn coach_plan(core: Core<'_>, exercise_id: String, level: Option<u32>) -> R<ExercisePlan> {
    run(&core, move |c| c.coach_plan(&exercise_id, level)).await
}

#[tauri::command]
async fn coach_demo(core: Core<'_>, plan: ExercisePlan) -> R<()> {
    run(&core, move |c| c.coach_demo(&plan)).await
}

#[tauri::command]
async fn coach_start_attempt(core: Core<'_>, plan: ExercisePlan) -> R<()> {
    run(&core, move |c| c.coach_start_attempt(plan)).await
}

#[tauri::command]
async fn coach_stop_attempt(core: Core<'_>, user_difficulty: Option<u8>, notes: Option<String>) -> R<AttemptResult> {
    run(&core, move |c| c.coach_stop_attempt(user_difficulty, notes)).await
}

#[tauri::command]
async fn coach_report_discomfort(core: Core<'_>, note: Option<String>) -> R<HealthStatus> {
    run(&core, move |c| c.coach_report_discomfort(note)).await
}

#[tauri::command]
async fn coach_rate_attempt(core: Core<'_>, id: String, difficulty: u8) -> R<()> {
    run(&core, move |c| c.rate_attempt(&id, difficulty)).await
}

#[tauri::command]
async fn range_start(core: Core<'_>, high: bool) -> R<()> {
    run(&core, move |c| c.range_start(high)).await
}

#[tauri::command]
async fn range_stop(core: Core<'_>) -> R<RangeResult> {
    run(&core, |c| c.range_stop()).await
}

#[tauri::command]
async fn skill_history(core: Core<'_>, skill: String) -> R<Vec<SkillHistoryPoint>> {
    run(&core, move |c| c.skill_history(&skill)).await
}

#[tauri::command]
async fn exercise_library(core: Core<'_>) -> R<Vec<serde_json::Value>> {
    run(&core, |c| Ok(c.exercise_library())).await
}

#[tauri::command]
async fn add_note(core: Core<'_>, text: String, skill: Option<String>) -> R<NoteEntry> {
    run(&core, move |c| c.add_note(&text, skill)).await
}

#[tauri::command]
async fn notes(core: Core<'_>) -> R<Vec<NoteEntry>> {
    run(&core, |c| c.notes()).await
}

#[tauri::command]
async fn delete_note(core: Core<'_>, id: String) -> R<()> {
    run(&core, move |c| c.delete_note(&id)).await
}

// --------------------------------------------------------------- studio

#[tauri::command]
async fn studio_new(core: Core<'_>, name: String, folder: Option<String>, sample_rate: Option<u32>) -> R<StudioState> {
    run(&core, move |c| c.studio_new(&name, folder.map(PathBuf::from), sample_rate)).await
}

#[tauri::command]
async fn studio_open(core: Core<'_>, path: String) -> R<StudioState> {
    run(&core, move |c| c.studio_open(&PathBuf::from(path))).await
}

#[tauri::command]
async fn studio_close(core: Core<'_>, save: bool) -> R<()> {
    run(&core, move |c| c.studio_close(save)).await
}

#[tauri::command]
async fn studio_save(core: Core<'_>) -> R<StudioState> {
    run(&core, |c| c.studio_save()).await
}

#[tauri::command]
async fn studio_state(core: Core<'_>) -> R<Option<StudioState>> {
    run(&core, |c| c.studio_state()).await
}

#[tauri::command]
async fn studio_edit(core: Core<'_>, op: EditOp, transient: Option<bool>) -> R<Option<StudioState>> {
    run(&core, move |c| c.studio_edit(op, transient.unwrap_or(false))).await
}

#[tauri::command]
async fn studio_undo(core: Core<'_>) -> R<StudioState> {
    run(&core, |c| c.studio_undo()).await
}

#[tauri::command]
async fn studio_redo(core: Core<'_>) -> R<StudioState> {
    run(&core, |c| c.studio_redo()).await
}

#[tauri::command]
async fn studio_play(core: Core<'_>, from: Option<u64>) -> R<()> {
    run(&core, move |c| c.studio_play(from)).await
}

#[tauri::command]
async fn studio_stop(core: Core<'_>) -> R<()> {
    run(&core, |c| c.studio_stop()).await
}

#[tauri::command]
async fn studio_locate(core: Core<'_>, position: u64) -> R<()> {
    run(&core, move |c| c.studio_locate(position)).await
}

#[tauri::command]
async fn studio_record(core: Core<'_>, from: Option<u64>) -> R<()> {
    run(&core, move |c| c.studio_record(from)).await
}

#[tauri::command]
async fn studio_stop_recording(core: Core<'_>) -> R<RecordStopResult> {
    run(&core, |c| c.studio_stop_recording()).await
}

#[tauri::command]
async fn studio_import(core: Core<'_>, files: Vec<String>, track: Option<Id>, position: u64) -> R<u64> {
    run(&core, move |c| c.studio_import(files.into_iter().map(PathBuf::from).collect(), track, position)).await
}

#[tauri::command]
async fn studio_export(core: Core<'_>, path: String, options: RenderOptions) -> R<u64> {
    run(&core, move |c| c.studio_export(PathBuf::from(path), options)).await
}

#[tauri::command]
async fn studio_asset_waveform(core: Core<'_>, asset: Id, start: u64, end: Option<u64>, buckets: usize) -> R<Vec<f32>> {
    run(&core, move |c| c.studio_asset_waveform(asset, start, end, buckets)).await
}

#[tauri::command]
async fn studio_revisions(core: Core<'_>) -> R<Vec<RevisionInfo>> {
    run(&core, |c| c.studio_revisions()).await
}

#[tauri::command]
async fn studio_load_revision(core: Core<'_>, id: i64) -> R<StudioState> {
    run(&core, move |c| c.studio_load_revision(id)).await
}

#[tauri::command]
async fn studio_backup(core: Core<'_>) -> R<String> {
    run(&core, |c| Ok(c.studio_backup()?.to_string_lossy().to_string())).await
}

#[tauri::command]
async fn studio_relink_asset(core: Core<'_>, asset: Id, file: String) -> R<StudioState> {
    run(&core, move |c| c.studio_relink_asset(asset, &PathBuf::from(file))).await
}

#[tauri::command]
async fn recent_projects(core: Core<'_>, query: Option<String>) -> R<Vec<ProjectEntry>> {
    run(&core, move |c| c.recent_projects(query.as_deref())).await
}

#[tauri::command]
async fn forget_project(core: Core<'_>, id: String) -> R<()> {
    run(&core, move |c| c.forget_project(&id)).await
}

// -------------------------------------------------------- effects/presets

#[tauri::command]
async fn effect_catalog(core: Core<'_>) -> R<serde_json::Value> {
    run(&core, |c| Ok(c.effect_catalog())).await
}

#[tauri::command]
async fn presets(core: Core<'_>, effect_kind: Option<String>) -> R<Vec<PresetEntry>> {
    run(&core, move |c| c.presets(effect_kind.as_deref())).await
}

#[tauri::command]
async fn save_preset(core: Core<'_>, effect_kind: String, name: String, params: BTreeMap<String, f32>) -> R<PresetEntry> {
    run(&core, move |c| c.save_preset(&effect_kind, &name, params)).await
}

#[tauri::command]
async fn delete_preset(core: Core<'_>, id: String) -> R<()> {
    run(&core, move |c| c.delete_preset(&id)).await
}

// ----------------------------------------------------------------- jobs

#[tauri::command]
async fn jobs_list(core: Core<'_>) -> R<Vec<JobInfo>> {
    run(&core, |c| Ok(c.jobs.list())).await
}

#[tauri::command]
async fn job_cancel(core: Core<'_>, id: u64) -> R<bool> {
    run(&core, move |c| Ok(c.jobs.cancel(id))).await
}

#[tauri::command]
async fn jobs_clear(core: Core<'_>) -> R<()> {
    run(&core, |c| {
        c.jobs.clear_finished();
        Ok(())
    })
    .await
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app_info,
        get_settings,
        update_settings,
        list_devices,
        restart_engine,
        engine_status,
        reset_clips,
        set_live_pitch,
        live_pitch,
        play_reference,
        stop_reference,
        preview_file,
        stop_preview,
        delete_training_history,
        backup_library,
        analyzer_start,
        analyzer_stop,
        capture_status,
        capture_cancel,
        recordings,
        recording_analysis,
        rename_recording,
        delete_recording,
        compare_recordings,
        waveform,
        coach_overview,
        coach_plan,
        coach_demo,
        coach_start_attempt,
        coach_stop_attempt,
        coach_report_discomfort,
        coach_rate_attempt,
        range_start,
        range_stop,
        skill_history,
        exercise_library,
        add_note,
        notes,
        delete_note,
        studio_new,
        studio_open,
        studio_close,
        studio_save,
        studio_state,
        studio_edit,
        studio_undo,
        studio_redo,
        studio_play,
        studio_stop,
        studio_locate,
        studio_record,
        studio_stop_recording,
        studio_import,
        studio_export,
        studio_asset_waveform,
        studio_revisions,
        studio_load_revision,
        studio_backup,
        studio_relink_asset,
        recent_projects,
        forget_project,
        effect_catalog,
        presets,
        save_preset,
        delete_preset,
        jobs_list,
        job_cancel,
        jobs_clear,
    ]
}
