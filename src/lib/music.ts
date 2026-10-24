// Pitch and time helpers (mirror crates/uta-dsp/src/tone.rs).

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export const midiToHz = (m: number) => 440 * Math.pow(2, (m - 69) / 12);
export const hzToMidi = (hz: number) => 69 + 12 * Math.log2(hz / 440);
export const centsBetween = (hz: number, ref: number) => 1200 * Math.log2(hz / ref);

export function noteName(midi: number): string {
  const n = Math.round(midi);
  const name = NAMES[((n % 12) + 12) % 12];
  return `${name}${Math.floor(n / 12) - 1}`;
}

/** Note name plus signed cents deviation from it, e.g. `A3 −12¢`. */
export function noteWithCents(midi: number): { name: string; cents: number } {
  const n = Math.round(midi);
  return { name: noteName(n), cents: Math.round((midi - n) * 100) };
}

export function formatCents(c: number): string {
  const r = Math.round(c);
  if (r === 0) return "±0¢";
  return `${r > 0 ? "+" : "−"}${Math.abs(r)}¢`;
}

export function formatTime(seconds: number, withMs = false): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  if (withMs) return `${m}:${s.toFixed(2).padStart(5, "0")}`;
  return `${m}:${Math.floor(s).toString().padStart(2, "0")}`;
}

export const gainToDb = (g: number) => (g <= 1e-6 ? -120 : 20 * Math.log10(g));
export const dbToGain = (db: number) => (db <= -120 ? 0 : Math.pow(10, db / 20));

export function formatDb(db: number, digits = 1): string {
  if (db <= -119) return "−∞";
  return `${db > 0 ? "+" : db < 0 ? "−" : ""}${Math.abs(db).toFixed(digits)}`;
}

export function relativeDate(iso: string): string {
  const d = new Date(iso);
  const diff = (Date.now() - d.getTime()) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  if (diff < 86400 * 7) return `${Math.floor(diff / 86400)} d ago`;
  return d.toLocaleDateString();
}

export function clamp(v: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, v));
}
