// TypeScript mirrors of the Rust IPC types (serde camelCase).
// Keep in sync with crates/uta-*/src and src-tauri/src/commands.rs.

export type Id = string;

export interface ErrorPayload {
  kind: string;
  message: string;
}

// ------------------------------------------------------------ settings/app

export interface VoiceRange {
  low: number;
  high: number;
}

export interface Settings {
  microphoneEnabled: boolean;
  inputDevice: string | null;
  outputDevice: string | null;
  sampleRate: number | null;
  bufferFrames: number | null;
  inputChannel: number;
  latencyOffsetMs: number;
  voiceRange: VoiceRange;
  voiceRangeMeasured: boolean;
  showRomaji: boolean;
  showKana: boolean;
  analysisHopMs: number;
  lightweightMode: boolean;
  autosaveMinutes: number;
  sessionLength: "short" | "long";
  pluginFolders: string[];
  referenceVolume: number;
  monitorInAnalyzer: boolean;
  firstRunDone: boolean;
}

export interface AppPaths {
  dataDir: string;
  libraryDb: string;
  recordingsDir: string;
  songsDir: string;
  backupsDir: string;
  logsDir: string;
  projectsDir: string;
}

export interface BackendInfo {
  kind: string;
  host: string;
  outputName: string | null;
  inputName: string | null;
  sampleRate: number;
  outputChannels: number;
  inputChannels: number;
  bufferFrames: number | null;
  notes: string[];
}

export interface AppInfo {
  version: string;
  platform: string;
  paths: AppPaths;
  settings: Settings;
  disclaimer: string;
  engine: BackendInfo | null;
  engineError: string | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
  maxChannels: number;
  sampleRates: number[];
  defaultSampleRate: number | null;
  bufferRange: [number, number] | null;
}

export interface DeviceList {
  host: string;
  inputs: DeviceInfo[];
  outputs: DeviceInfo[];
}

export interface MeterValue {
  peak: number;
  rms: number;
  clipped: boolean;
}

export interface EngineStatus {
  running: boolean;
  backend: BackendInfo;
  playing: boolean;
  recording: boolean;
  position: number;
  sampleRate: number;
  recordFrames: number;
  previewPlaying: boolean;
  previewPosition: number;
  dspLoad: number;
  inputUnderruns: number;
  inputOverflows: number;
  streamUnderruns: number;
  deviceXruns: number;
  deviceLost: boolean;
  inputLatencyMs: number;
  outputLatencyMs: number;
  master: [MeterValue, MeterValue];
  tracks: [MeterValue, MeterValue][];
  inputs: MeterValue[];
}

// ---------------------------------------------------------------- analysis

export interface PitchFrame {
  time: number;
  hz: number | null;
  candidateHz: number | null;
  confidence: number;
  rmsDb: number;
}

export interface LiveFrame extends PitchFrame {
  seq: number;
}

export interface DetectedNote {
  start: number;
  end: number;
  midi: number;
  spreadCents: number;
  meanConfidence: number;
}

export interface Vibrato {
  rateHz: number;
  extentCents: number;
  regularity: number;
}

export interface SustainAnalysis {
  start: number;
  duration: number;
  midi: number;
  targetErrorCents: number | null;
  driftCentsPerSecond: number;
  driftCents: number;
  wobbleCents: number;
  vibrato: Vibrato | null;
}

export type QualityLevel = "good" | "fair" | "poor";

export interface RecordingQuality {
  level: QualityLevel;
  issues: string[];
  noiseFloorDb: number | null;
  signalDb: number | null;
  snrDb: number | null;
  voicedRatio: number;
  meanConfidence: number | null;
}

export interface TargetNote {
  midi: number;
  start: number;
  duration: number;
}

export interface NoteResult {
  target: TargetNote;
  sungMidi: number | null;
  errorCents: number | null;
  octaveShift: number;
  onsetErrorMs: number | null;
  coverage: number;
  wobbleCents: number | null;
  measurable: boolean;
}

export interface TargetComparison {
  notes: NoteResult[];
  measurableNotes: number;
  meanAbsErrorCents: number | null;
  within25Cents: number | null;
  within50Cents: number | null;
  meanOnsetErrorMs: number | null;
  onsetSpreadMs: number | null;
}

export interface PhraseAnalysis {
  analysisVersion: number;
  duration: number;
  sampleRate: number;
  hopSeconds: number;
  frames: PitchFrame[];
  notes: DetectedNote[];
  pitch: {
    voicedRatio: number;
    voicedSeconds: number;
    medianMidi: number | null;
    lowMidi: number | null;
    highMidi: number | null;
    meanConfidence: number | null;
  };
  dynamics: {
    peakDb: number;
    voicedRmsDb: number | null;
    clippedSamples: number;
    clipRatio: number;
    dynamicRangeDb: number | null;
    trendDbPerSecond: number | null;
  };
  spectral: {
    medianCentroidHz: number | null;
    centroidSpreadHz: number | null;
    medianHighRatioDb: number | null;
    medianH1H2Db: number | null;
    meanPeriodicity: number | null;
  };
  sustain: SustainAnalysis | null;
  quality: RecordingQuality;
  targets: TargetComparison | null;
}

export interface Suggestion {
  exerciseId: string;
  title: string;
  reason: string;
}

export interface Feedback {
  headline: string;
  wentWell: string[];
  primaryOpportunity: string | null;
  suggestion: Suggestion | null;
  retryGoal: string | null;
  uncertainty: string | null;
  safetyReminder: string | null;
}

export interface RecordingEntry {
  id: string;
  path: string;
  createdAt: string;
  duration: number;
  source: string;
  title: string;
  exerciseId: string | null;
  summary: Record<string, unknown>;
}

export interface AnalyzerResult {
  recording: RecordingEntry;
  analysis: PhraseAnalysis;
  feedback: Feedback;
}

export interface CaptureStatus {
  active: boolean;
  kind: string | null;
  seconds: number;
  inputDevice: string | null;
}

export interface ContourComparison {
  meanAbsPitchDiffCents: number | null;
  medianPitchDiffCents: number | null;
  medianTimingOffsetMs: number;
  p90AbsTimingOffsetMs: number;
  voicedOverlap: number;
  path: [number, number][];
}

export interface MetricDelta {
  key: string;
  label: string;
  a: number | null;
  b: number | null;
  verdict: "better" | "worse" | "same" | "n/a";
}

export interface RecordingComparison {
  a: RecordingEntry;
  b: RecordingEntry;
  contour: ContourComparison | null;
  metrics: MetricDelta[];
  notes: string[];
}

// ------------------------------------------------------------------- coach

export type ExerciseKind =
  | "pitchMatch" | "scale" | "sustain" | "siren" | "octaveSlide" | "swell" | "rhythm" | "onset"
  | "vowels" | "phraseLength" | "articulation" | "colorContrast" | "phraseShape" | "arpeggio"
  | "vibrato" | "texture" | "moraTiming" | "melodyVowels" | "warmup";

export interface GuideNote {
  midi: number;
  start: number;
  duration: number;
}

export interface ExercisePlan {
  exerciseId: string;
  skillId: string;
  kind: ExerciseKind;
  level: number;
  title: string;
  objective: string;
  steps: string[];
  safety: string | null;
  demo: GuideNote[];
  guide: GuideNote[];
  guideVolume: number;
  clickBpm: number | null;
  targets: TargetNote[];
  syllables: string[];
  beats: number[];
  recordSeconds: number;
  octaveTolerant: boolean;
  criteria: Record<string, number>;
  noteLabels: string[];
}

export type Outcome = "success" | "partial" | "needsWork" | "inconclusive";

export interface CheckResult {
  key: string;
  label: string;
  value: string;
  pass: boolean;
  score: number;
}

export interface Evaluation {
  outcome: Outcome;
  score: number;
  confidence: number;
  checks: CheckResult[];
  metrics: Record<string, number>;
  feedback: Feedback;
}

export type Decision =
  | { type: "levelUp"; to: number }
  | { type: "keep" }
  | { type: "simplify"; to: number }
  | { type: "requestCleanRecording" }
  | { type: "stop" };

export interface AttemptEntry {
  id: string;
  createdAt: string;
  exerciseId: string;
  skillId: string;
  level: number;
  outcome: string;
  score: number | null;
  confidence: number | null;
  metrics: Record<string, number>;
  recordingId: string | null;
  userDifficulty: number | null;
  discomfort: boolean;
  notes: string;
}

export interface AttemptResult {
  attempt: AttemptEntry;
  evaluation: Evaluation;
  decision: Decision;
  decisionText: string;
  newLevel: number;
  nextReview: string | null;
  recording: RecordingEntry;
}

export interface SkillView {
  id: string;
  stage: number;
  stageTitle: string;
  title: string;
  summary: string;
  level: number;
  unlocked: boolean;
  requires: string[];
  attempts: number;
  lastOutcome: string | null;
  exercises: string[];
}

export interface ReviewView {
  exerciseId: string;
  exerciseTitle: string;
  skillId: string;
  due: string;
  overdue: boolean;
  boxIndex: number;
}

export interface HealthStatus {
  restingUntil: string | null;
  reportsLast14Days: number;
  cleanWindow: boolean;
  recommendProfessional: boolean;
  message: string | null;
}

export interface Recommendation {
  skill: string;
  exercise: string;
  level: number;
  reason: string;
}

export interface CoachOverview {
  skills: SkillView[];
  reviews: ReviewView[];
  health: HealthStatus;
  recommendation: Recommendation | null;
  session: { minutes: number; items: Recommendation[] };
  recent: AttemptEntry[];
  voiceRange: VoiceRange;
  voiceRangeMeasured: boolean;
  currentStage: number;
  disclaimer: string;
  practiceDaysLast14: number;
}

export interface ExerciseInfo {
  id: string;
  skill: string;
  kind: ExerciseKind;
  title: string;
  objective: string;
  steps: string[];
  warmup: string | null;
  safety: string | null;
}

export interface RangeResult {
  kind: string;
  midi: number | null;
  note: string | null;
  voiceRange: VoiceRange;
  message: string;
}

export interface SkillHistoryPoint {
  at: string;
  level: number;
  outcome: string;
  score: number | null;
  exerciseId: string;
}

export interface NoteEntry {
  id: string;
  createdAt: string;
  skillId: string | null;
  text: string;
}

// ------------------------------------------------------------------ studio

export type MonitorMode = "off" | "armed" | "on";

export interface Clip {
  id: Id;
  assetId: Id;
  name: string;
  position: number;
  sourceOffset: number;
  length: number;
  gainDb: number;
  fadeIn: number;
  fadeOut: number;
  muted: boolean;
}

export interface CompSegment {
  start: number;
  end: number;
  take: Id;
}

export interface TakeGroup {
  id: Id;
  takes: Id[];
  comp: CompSegment[];
}

export type BuiltinKind = "eq" | "compressor" | "gate" | "reverb" | "delay" | "utility";

export interface PluginRef {
  format: string;
  uid: string;
  name: string;
  vendor: string;
  version: string;
  path: string;
}

export type EffectKind =
  | { type: "builtin"; kind: BuiltinKind; params: Record<string, number> }
  | { type: "plugin"; plugin: PluginRef; state: string | null; params: Record<string, number> };

export interface EffectSlot {
  id: Id;
  bypass: boolean;
  effect: EffectKind;
}

export interface Track {
  id: Id;
  name: string;
  color: string;
  gainDb: number;
  pan: number;
  mute: boolean;
  solo: boolean;
  armed: boolean;
  monitor: MonitorMode;
  inputChannel: number;
  inputStereo: boolean;
  clips: Clip[];
  effects: EffectSlot[];
  takeGroups: TakeGroup[];
  height: number;
}

export interface Asset {
  id: Id;
  file: string;
  originalName: string;
  kind: "recording" | "import" | "render";
  sampleRate: number;
  channels: number;
  frames: number;
  createdAt: string;
}

export interface Region {
  start: number;
  end: number;
}

export interface Marker {
  id: Id;
  position: number;
  label: string;
}

export interface Metronome {
  enabled: boolean;
  volumeDb: number;
  countInBars: number;
}

export interface Project {
  formatVersion: number;
  id: Id;
  name: string;
  createdAt: string;
  modifiedAt: string;
  sampleRate: number;
  tempoBpm: number;
  timeSignature: [number, number];
  tracks: Track[];
  master: { gainDb: number; effects: EffectSlot[] };
  assets: Asset[];
  loopRegion: Region | null;
  markers: Marker[];
  metronome: Metronome;
  notes: string;
  linkedAttempts: Id[];
}

export interface AssetCheck {
  assetId: Id;
  file: string;
  status: { status: "ok" | "missing" | "mismatch" | "unreadable" | "unsafePath"; reason?: string };
}

export interface RevisionInfo {
  id: number;
  kind: "save" | "autosave";
  createdAt: string;
  formatVersion: number;
}

export interface StudioState {
  project: Project;
  path: string;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  assetChecks: AssetCheck[];
  warnings: string[];
  recording: boolean;
  newerAutosave: RevisionInfo | null;
  /** Status of every third-party plugin slot in the project. */
  pluginSlots: SlotStatus[];
}

// ------------------------------------------------------------- plugins

export interface PluginInfo {
  uid: string;
  name: string;
  vendor: string;
  version: string;
  category: string;
  subCategories: string;
  isInstrument: boolean;
  audioInputs: number;
  audioOutputs: number;
}

export interface PluginBundle {
  path: string;
  enabled: boolean;
  error: string | null;
  plugins: PluginInfo[];
  scannedAt: string;
  missing: boolean;
}

export interface PluginCatalog {
  available: boolean;
  helperPath: string | null;
  defaultFolders: string[];
  userFolders: string[];
  bundles: PluginBundle[];
}

export type InstanceStatus = "running" | "crashed" | "missing" | "failed" | "disabled" | "unavailable";

export interface SlotStatus {
  slot: Id;
  track: Id | null;
  name: string;
  vendor: string;
  status: InstanceStatus;
  message: string | null;
  latencyFrames: number;
  lateBlocks: number;
  warnings: string[];
}

export interface ParamDesc {
  id: number;
  title: string;
  units: string;
  defaultNormalized: number;
  stepCount: number;
  canAutomate: boolean;
  readOnly: boolean;
  isBypass: boolean;
}

export interface ParamValue {
  id: number;
  value: number;
  display: string;
}

export interface PluginParams {
  params: ParamDesc[];
  values: ParamValue[];
}

export interface RecordStopResult {
  clips: Id[];
  latencyFrames: number;
  droppedFrames: number;
  warnings: string[];
}

export interface TrackUpdate {
  name?: string;
  color?: string;
  gainDb?: number;
  pan?: number;
  mute?: boolean;
  solo?: boolean;
  armed?: boolean;
  monitor?: MonitorMode;
  inputChannel?: number;
  inputStereo?: boolean;
  height?: number;
}

export interface ClipUpdate {
  name?: string;
  gainDb?: number;
  fadeIn?: number;
  fadeOut?: number;
  muted?: boolean;
}

export type EditOp =
  | { op: "addTrack"; name?: string | null }
  | { op: "removeTrack"; track: Id }
  | { op: "moveTrack"; track: Id; index: number }
  | { op: "updateTrack"; track: Id; update: TrackUpdate }
  | { op: "moveClip"; clip: Id; position: number; track?: Id | null }
  | { op: "trimClip"; clip: Id; start?: number | null; end?: number | null }
  | { op: "splitClip"; clip: Id; at: number }
  | { op: "duplicateClip"; clip: Id; position?: number | null }
  | { op: "deleteClip"; clip: Id }
  | { op: "updateClip"; clip: Id; update: ClipUpdate }
  | { op: "setComp"; track: Id; group: Id; start: number; end: number; take: Id }
  | { op: "flattenComp"; track: Id; group: Id }
  | { op: "insertEffect"; track: Id | null; effect: BuiltinKind; index?: number | null }
  | { op: "insertPlugin"; track: Id | null; plugin: PluginRef; index?: number | null }
  | { op: "removeEffect"; track: Id | null; slot: Id }
  | { op: "moveEffect"; track: Id | null; slot: Id; index: number }
  | { op: "setEffectBypass"; track: Id | null; slot: Id; bypass: boolean }
  | { op: "setEffectParam"; track: Id | null; slot: Id; param: string; value: number }
  | { op: "setEffectParams"; track: Id | null; slot: Id; params: Record<string, number> }
  | { op: "setPluginState"; track: Id | null; slot: Id; state?: string | null; params?: Record<string, number> | null }
  | { op: "relinkPlugin"; track: Id | null; slot: Id; plugin: PluginRef }
  | { op: "setPluginParam"; track: Id | null; slot: Id; id: number; value: number }
  | { op: "setMasterGain"; gainDb: number }
  | { op: "setLoop"; start?: number | null; end?: number | null }
  | { op: "setTempo"; bpm: number }
  | { op: "setMetronome"; metronome: Metronome }
  | { op: "renameProject"; name: string }
  | { op: "setNotes"; notes: string }
  | { op: "addMarker"; position: number; label: string }
  | { op: "removeMarker"; id: Id };

export interface ProjectEntry {
  id: string;
  name: string;
  path: string;
  createdAt: string;
  modifiedAt: string;
  lastOpenedAt: string;
  exists: boolean;
}

export type BitDepth = "int16" | "int24" | "float32";

export interface RenderOptions {
  start: number;
  end: number;
  tailSeconds: number;
  sampleRate: number | null;
  depth: BitDepth;
  channels: number;
  dither: boolean;
}

export interface RenderReport {
  path: string;
  frames: number;
  sampleRate: number;
  channels: number;
  durationSeconds: number;
  peakDb: number;
  clippedSamples: number;
  verified: boolean;
  warnings: string[];
}

export type ParamScale =
  | { type: "linear" }
  | { type: "log" }
  | { type: "toggle" }
  | { type: "choice"; labels: string[] };

export interface ParamInfo {
  id: string;
  name: string;
  unit: string;
  min: number;
  max: number;
  default: number;
  scale: ParamScale;
}

export interface EffectCatalogEntry {
  id: BuiltinKind;
  name: string;
  params: ParamInfo[];
}

export interface PresetEntry {
  id: string;
  effectKind: string;
  name: string;
  params: Record<string, number>;
  createdAt: string;
}

export type JobState =
  | { state: "queued" }
  | { state: "running" }
  | { state: "done"; result: unknown }
  | { state: "failed"; error: string }
  | { state: "cancelled" };

export type JobInfo = {
  id: number;
  kind: string;
  label: string;
  progress: number;
} & JobState;
