// Small helpers shared across the UI.

export const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
export const clamp01 = (v: number) => clamp(v, 0, 1);

/** Metronome tempo range (the engine accepts the same). */
export const BPM_MIN = 30;
export const BPM_MAX = 300;
export const clampBpm = (bpm: number) => Math.round(clamp(bpm, BPM_MIN, BPM_MAX));

/** Lowest level on faders; at or below it reads "-∞". */
export const FLOOR_DB = -60;

/** "+3.5" / "-12.0" / "-∞" (no unit). */
export const formatDb = (db: number) => (db <= FLOOR_DB + 0.1 ? "-∞" : `${db > 0 ? "+" : ""}${db.toFixed(1)}`);

/** "C", "L40", "R100". */
export const formatPan = (p: number) => (Math.abs(p) < 0.02 ? "C" : `${p < 0 ? "L" : "R"}${Math.round(Math.abs(p) * 100)}`);

/** "just now", "5 min ago", "3 h ago", then the date. */
export function timeAgo(secs: number | null) {
  if (!secs) return "never";
  const d = Date.now() / 1000 - secs;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return new Date(secs * 1000).toLocaleDateString();
}

/** Is a key event aimed at a text field (or a dialog), where shortcuts must stay out? */
export const isTyping = (e: Event) =>
  !!(e.target as HTMLElement | null)?.closest?.("input, textarea, select, [contenteditable='true'], .modal");

/** How in tune a note is (CSS state class): ±3¢ "in-tune", ±15¢ "close", else "off";
 *  "idle" when there's no note. */
export const tuneState = (cents: number | null) =>
  cents === null ? "idle" : Math.abs(cents) < 3 ? "in-tune" : Math.abs(cents) < 15 ? "close" : "off";

/** The tuner strip: 21 ticks, 5¢ apart, the centre one at index 10. */
export const TUNER_TICKS = Array.from({ length: 21 }, (_, i) => i);
export const tickLit = (i: number, cents: number) => Math.abs((i - 10) * 5 - clamp(cents, -50, 50)) <= 2.5;
export const formatCents = (cents: number) => `${cents > 0 ? "+" : ""}${cents.toFixed(1)}¢`;
