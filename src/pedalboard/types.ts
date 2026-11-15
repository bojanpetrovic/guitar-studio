// Pedalboard data model. Stored inside each preset (the engine keeps it opaque).

export type WidgetType = "footswitch" | "knob" | "fader" | "selector" | "meter" | "label" | "panel" | "tuner";

/** What a control is wired to. Node ids are rig node ids; param ids are the plugin's stable ids. */
export type Binding =
  | { kind: "none" }
  | { kind: "param"; node: number; paramId: number; paramName: string }
  | { kind: "bypass"; node: number }
  | { kind: "level"; node: number }
  /** One strip of a Mixer node: volume, pan, or mute (footswitch). */
  | { kind: "mix"; node: number; source: number; field: "gainDb" | "pan" | "mute" }
  | { kind: "meter"; node: number | "master" }
  | { kind: "metronome" }
  | { kind: "tap" }
  | { kind: "bpm" }
  | { kind: "tuner" };

export type BindingKind = Binding["kind"];

export interface WidgetOptions {
  /** Footswitch: fire on press, release on let go. */
  momentary?: boolean;
  /** Footswitch on a parameter: values for on / off (native units). */
  onValue?: number;
  offValue?: number;
  /** Knob / fader: narrow the usable range (native units). */
  min?: number;
  max?: number;
  /** Selector: labels for each position (evenly spread over the parameter range). */
  labels?: string[];
  /** Label: text size in px. */
  fontSize?: number;
  /** Pedal: the enclosure's finish (see PEDAL_FINISHES; "classic" when unset). */
  finish?: PedalFinish;
  /**
   * Keyboard key (KeyboardEvent.code, e.g. "KeyA") that plays this control in play
   * mode: switches press; knobs / faders / selectors are selected, then nudged with ↑ / ↓.
   */
  key?: string;
}

export interface Widget {
  id: string;
  type: WidgetType;
  x: number;
  y: number;
  w: number;
  h: number;
  label: string;
  /** Accent (LED, knob arc) or enclosure color for panels. */
  color: string;
  binding: Binding;
  options: WidgetOptions;
}

/** Pedal enclosure finishes (styled in styles.css as `.finish-<id>`). Animated ones run
 *  while the pedal is on (when it follows a plugin's on/off) and hold still with Reduce
 *  Motion on. */
export const PEDAL_FINISHES = [
  { id: "classic", label: "Classic" },
  { id: "brushed", label: "Brushed" },
  { id: "hammered", label: "Hammered" },
  { id: "vintage", label: "Vintage" },
  { id: "flake", label: "Flake" },
  { id: "neon", label: "Neon", animated: true },
  { id: "holo", label: "Holo", animated: true },
  { id: "scope", label: "Scope", animated: true },
  { id: "lava", label: "Lava", animated: true },
] as const;
export type PedalFinish = (typeof PEDAL_FINISHES)[number]["id"];

/** Board backgrounds (styled in styles.css as `.surface-<id>`). Animated ones draw on
 *  a `.pb-surface` layer under the controls (`particles`: on a canvas, see Surface.tsx),
 *  and stand still with Reduce Motion on. */
export const SURFACES = [
  { id: "slate", label: "Slate" },
  { id: "carbon", label: "Carbon" },
  { id: "wood", label: "Wood" },
  { id: "walnut", label: "Walnut" },
  { id: "maple", label: "Maple" },
  { id: "mahogany", label: "Mahogany" },
  { id: "rosewood", label: "Rosewood" },
  { id: "ebony", label: "Ebony" },
  { id: "planks", label: "Oak planks" },
  { id: "barnwood", label: "Barnwood" },
  { id: "charred", label: "Charred" },
  { id: "steel", label: "Steel" },
  { id: "tolex", label: "Tolex" },
  { id: "tweed", label: "Tweed" },
  { id: "velvet", label: "Velvet" },
  { id: "blueprint", label: "Blueprint" },
  { id: "aurora", label: "Aurora", animated: true },
  { id: "lava", label: "Lava", animated: true },
  { id: "synthwave", label: "Synthwave", animated: true },
  { id: "cosmos", label: "Cosmos", animated: true },
  { id: "plexus", label: "Plexus", animated: true, particles: "plexus" },
  { id: "embers", label: "Embers", animated: true, particles: "embers" },
  { id: "frost", label: "Frost", animated: true, particles: "frost" },
  { id: "toxic", label: "Toxic", animated: true, particles: "toxic" },
  { id: "ultraviolet", label: "Ultraviolet", animated: true, particles: "ultraviolet" },
  { id: "sakura", label: "Sakura", animated: true, particles: "sakura" },
  { id: "bokeh", label: "Bokeh", animated: true, particles: "bokeh" },
] as const;

export type BoardSurface = (typeof SURFACES)[number]["id"];

export interface Board {
  version: 1;
  width: number;
  height: number;
  surface: BoardSurface;
  widgets: Widget[];
  /** Scene buttons under the board (on unless turned off). */
  showScenes?: boolean;
}

export const GRID = 10;

/** Every board has this size, so presets look alike on stage and on the phone (and the
 *  stage window doesn't jump when the preset changes). */
export const BOARD_W = 1200;
export const BOARD_H = 560;

const emptyBoard = (): Board => ({ version: 1, width: BOARD_W, height: BOARD_H, surface: "slate", widgets: [] });

/**
 * Put a board at the fixed size. Controls that reach past it (boards from before the size
 * was fixed, or "Build from rig" for a long chain) are scaled down together to fit.
 */
export function fitBoard(b: Board): Board {
  const right = Math.max(0, ...b.widgets.map((w) => w.x + w.w));
  const bottom = Math.max(0, ...b.widgets.map((w) => w.y + w.h));
  const k = Math.min(1, BOARD_W / right || 1, BOARD_H / bottom || 1);
  const widgets =
    k === 1
      ? b.widgets
      : b.widgets.map((w) => {
          const m = WIDGET_META[w.type];
          const ww = Math.min(BOARD_W, Math.max(m?.minW ?? 0, Math.round(w.w * k)));
          const hh = Math.min(BOARD_H, Math.max(m?.minH ?? 0, Math.round(w.h * k)));
          return {
            ...w,
            w: ww,
            h: hh,
            x: Math.max(0, Math.min(BOARD_W - ww, Math.round(w.x * k))),
            y: Math.max(0, Math.min(BOARD_H - hh, Math.round(w.y * k))),
          };
        });
  return { ...b, width: BOARD_W, height: BOARD_H, widgets };
}

/** Accept whatever is stored in a preset; anything unrecognisable becomes an empty board. */
export function parseBoard(raw: unknown): Board {
  const b = raw as Partial<Board> | null;
  if (!b || b.version !== 1 || !Array.isArray(b.widgets)) return emptyBoard();
  return fitBoard({ ...emptyBoard(), ...b, widgets: b.widgets.filter((w) => w && typeof w.id === "string") } as Board);
}

export const ACCENTS = ["#ffa630", "#ff5d5d", "#3ddc97", "#7aa2ff", "#c38bff", "#ffd84a", "#5ee0f0", "#f2f2f2"];
/** Classic stompbox enclosure colors. */
export const ENCLOSURES = ["#e0602b", "#c9302c", "#2f6fd6", "#2e9d5b", "#7c4dce", "#e8b923", "#9aa3ad", "#24272e"];

export const WIDGET_META: Record<
  WidgetType,
  { name: string; w: number; h: number; minW: number; minH: number; bindings: BindingKind[] }
> = {
  footswitch: { name: "Footswitch", w: 100, h: 130, minW: 70, minH: 100, bindings: ["bypass", "param", "mix", "metronome", "tap", "tuner", "none"] },
  knob: { name: "Knob", w: 90, h: 120, minW: 60, minH: 80, bindings: ["param", "level", "mix", "bpm", "none"] },
  fader: { name: "Fader", w: 70, h: 200, minW: 50, minH: 120, bindings: ["param", "level", "mix", "bpm", "none"] },
  selector: { name: "Selector", w: 140, h: 130, minW: 90, minH: 60, bindings: ["param", "none"] },
  meter: { name: "Meter", w: 50, h: 180, minW: 30, minH: 80, bindings: ["meter"] },
  label: { name: "Label", w: 200, h: 44, minW: 40, minH: 24, bindings: [] },
  // A pedal's binding only drives its LED (and whether an animated finish runs).
  panel: { name: "Pedal", w: 280, h: 340, minW: 100, minH: 100, bindings: ["none", "bypass"] },
  tuner: { name: "Tuner", w: 240, h: 110, minW: 160, minH: 80, bindings: [] },
};

export const BINDING_LABEL: Record<BindingKind, string> = {
  none: "Nothing",
  param: "Plugin parameter",
  bypass: "On/off (or output mute)",
  level: "Level (dB)",
  mix: "Mixer channel",
  meter: "Signal level",
  metronome: "Metronome on/off",
  tap: "Tap tempo",
  bpm: "Tempo (BPM)",
  tuner: "Tuner on/off",
};

let counter = 0;
export const newId = () => `w${Date.now().toString(36)}${(counter++).toString(36)}`;

export function makeWidget(type: WidgetType, x: number, y: number): Widget {
  const m = WIDGET_META[type];
  const defaults: Partial<Widget> = {
    footswitch: { label: "Stomp", binding: { kind: "none" } as Binding },
    knob: { label: "Knob" },
    fader: { label: "Fader" },
    selector: { label: "Mode", options: { labels: ["A", "B", "C"] } },
    meter: { label: "", binding: { kind: "meter", node: "master" } as Binding },
    label: { label: "My Rig", options: { fontSize: 22 } },
    panel: { label: "PEDAL", color: ENCLOSURES[0] },
    tuner: { label: "Tuner" },
  }[type];
  return {
    id: newId(),
    type,
    x: snap(x),
    y: snap(y),
    w: m.w,
    h: m.h,
    color: ACCENTS[0],
    binding: { kind: "none" },
    options: {},
    ...defaults,
    label: defaults.label ?? m.name,
  } as Widget;
}

export const snap = (v: number) => Math.round(v / GRID) * GRID;

/** Panels render underneath everything else. */
export const drawOrder = (ws: Widget[]) => [...ws.filter((w) => w.type === "panel"), ...ws.filter((w) => w.type !== "panel")];

// ---------------------------------------------------------------- keyboard keys

/** Controls a key can be assigned to. */
export const KEYABLE: WidgetType[] = ["footswitch", "knob", "fader", "selector"];

/** Keys the app already uses everywhere (code → what for). */
export function reservedKey(code: string): string | null {
  if (code === "Space") return "starts / stops the metronome";
  if (code === "ArrowUp" || code === "ArrowDown") return "tempo, and adjusting a held control";
  if (["ArrowLeft", "ArrowRight", "PageUp", "PageDown", "BracketLeft", "BracketRight"].includes(code))
    return "switches presets on stage";
  if (/^(Digit|Numpad)[1-9]$/.test(code)) return "switches scenes";
  if (code === "Escape") return "leaves the stage";
  if (["Tab", "Enter", "NumpadEnter", "Backspace", "Delete"].includes(code)) return "used for editing";
  if (/^(Shift|Control|Alt|Meta|OS|CapsLock|Fn)/.test(code)) return "a modifier key";
  return null;
}

/** Short label for a key code ("KeyA" → "A", "Digit0" → "0", "Semicolon" → ";"). */
export function keyLabel(code: string): string {
  const named: Record<string, string> = {
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backslash: "\\",
    Minus: "-",
    Equal: "=",
    Backquote: "`",
    IntlBackslash: "§",
  };
  if (named[code]) return named[code];
  return code.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Numpad/, "Num ");
}
