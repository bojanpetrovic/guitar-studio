// App-wide color themes. The palettes live in styles.css (`:root[data-theme="…"]`);
// this module picks one, remembers it, and keeps the native window in step.
import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_DESKTOP } from "./transport";

export type ThemeId = "amber" | "midnight" | "neon" | "graphite" | "daylight" | "paper";
export type ThemeChoice = ThemeId | "system";

export interface Theme {
  id: ThemeId;
  name: string;
  blurb: string;
  mode: "dark" | "light";
  /** Preview colors for the picker; must match styles.css. */
  swatch: { bg: string; panel: string; line: string; accent: string; accent2: string; text: string };
}

export const THEMES: Theme[] = [
  {
    id: "amber",
    name: "Amber",
    blurb: "Warm tube glow",
    mode: "dark",
    swatch: { bg: "#0b0c10", panel: "#191c24", line: "#2a2e38", accent: "#ffa630", accent2: "#ff6a1f", text: "#eceef3" },
  },
  {
    id: "midnight",
    name: "Midnight",
    blurb: "Cool blue, late session",
    mode: "dark",
    swatch: { bg: "#070b14", panel: "#131c31", line: "#26324c", accent: "#4cc9f0", accent2: "#4361ee", text: "#e6edf8" },
  },
  {
    id: "neon",
    name: "Neon",
    blurb: "Synthwave pink & violet",
    mode: "dark",
    swatch: { bg: "#0d0716", panel: "#1e1331", line: "#34244d", accent: "#ff4fa3", accent2: "#a855f7", text: "#f3ebff" },
  },
  {
    id: "graphite",
    name: "Graphite",
    blurb: "Neutral grey, lime signal",
    mode: "dark",
    swatch: { bg: "#0d0d0e", panel: "#1e1e20", line: "#333336", accent: "#c6f432", accent2: "#8fd400", text: "#ededee" },
  },
  {
    id: "daylight",
    name: "Daylight",
    blurb: "Bright and crisp",
    mode: "light",
    swatch: { bg: "#edf0f4", panel: "#ffffff", line: "#d5dae3", accent: "#f07a12", accent2: "#e8590c", text: "#161a22" },
  },
  {
    id: "paper",
    name: "Paper",
    blurb: "Warm cream, teal ink",
    mode: "light",
    swatch: { bg: "#f2eee5", panel: "#fbf9f4", line: "#ddd4c3", accent: "#0f9b8e", accent2: "#0b7a8f", text: "#29231a" },
  },
];

const KEY = "gs.theme";
const DEFAULT: ThemeChoice = "amber";
/** What "Match system" maps to. */
const SYSTEM_DARK: ThemeId = "amber";
const SYSTEM_LIGHT: ThemeId = "daylight";

const byId = (id: string) => THEMES.find((t) => t.id === id);
const darkQuery = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

function readChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "system" || (v && byId(v))) return v as ThemeChoice;
  } catch {
    /* storage unavailable: use the default */
  }
  return DEFAULT;
}

function resolveTheme(choice: ThemeChoice): Theme {
  if (choice === "system") return byId(darkQuery?.matches === false ? SYSTEM_LIGHT : SYSTEM_DARK)!;
  return byId(choice) ?? byId(DEFAULT)!;
}

let state = { choice: readChoice(), theme: resolveTheme(readChoice()) };
const listeners = new Set<() => void>();
let fadeTimer: number | undefined;

function apply(theme: Theme, animate: boolean) {
  const root = document.documentElement;
  if (animate) {
    root.classList.add("theme-fade");
    window.clearTimeout(fadeTimer);
    fadeTimer = window.setTimeout(() => root.classList.remove("theme-fade"), 320);
  }
  root.dataset.theme = theme.id;

  // Native window: background shows during resizes/startup, theme drives macOS chrome.
  if (IS_DESKTOP) {
    const win = getCurrentWindow();
    syncWindowBackground();
    win.setTheme(theme.mode).catch(() => {});
  }
}

/** Stage mode makes the window see-through so the pedalboard floats on the desktop. */
let windowTransparent = false;

function syncWindowBackground() {
  if (!IS_DESKTOP) return;
  const color: string | [number, number, number, number] = windowTransparent ? [0, 0, 0, 0] : state.theme.swatch.bg;
  getCurrentWindow()
    .setBackgroundColor(color)
    .catch(() => {});
}

export function setWindowTransparent(on: boolean) {
  windowTransparent = on;
  syncWindowBackground();
}

function update(choice: ThemeChoice, animate: boolean) {
  const theme = resolveTheme(choice);
  const changed = theme.id !== state.theme.id;
  state = { choice, theme };
  if (changed || !animate) apply(theme, animate);
  listeners.forEach((l) => l());
}

export function setThemeChoice(choice: ThemeChoice) {
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    /* not persisted, still applied for this session */
  }
  update(choice, true);
}

/** Apply the saved theme before the first render (no flash of the default palette). */
export function initTheme() {
  apply(state.theme, false);
  darkQuery?.addEventListener("change", () => state.choice === "system" && update("system", true));
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

export function useTheme() {
  return useSyncExternalStore(subscribe, () => state);
}

/** Step through the themes (⌘⇧T). */
export function cycleTheme(dir: 1 | -1 = 1) {
  const i = THEMES.findIndex((t) => t.id === state.theme.id);
  setThemeChoice(THEMES[(i + dir + THEMES.length) % THEMES.length].id);
}
