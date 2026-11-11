// Shared UI state: the context App provides and the hooks components read it with.
// Split in four so that a component only re-renders for what it reads: the actions
// never change, the catalog and audio status rarely do, the full studio view often
// (every fader move).
import { createContext, useContext } from "react";
import type { AudioStatus, Catalog, MetronomeSettings, PluginDescriptor, PresetList, StripField, StudioView } from "./api";
import type { Toast } from "./components/Toasts";

export type Pos = { x: number; y: number };

export interface StudioActions {
  addPlugin: (p: PluginDescriptor, pos?: Pos) => Promise<void>;
  addGain: (pos?: Pos) => Promise<void>;
  addMixer: (pos?: Pos) => Promise<void>;
  setMixerStrip: (id: number, source: number, field: StripField, value: number) => void;
  remove: (id: number) => Promise<void>;
  /** A node was dragged to a new place on the canvas. */
  moveNode: (id: number, x: number, y: number) => void;
  /** Open the "Replace with…" picker for a plugin node. */
  pickReplacement: (id: number) => void;
  /** Swap the plugin in node `id` for `p`, keeping its cables, place and board controls. */
  replacePlugin: (id: number, p: PluginDescriptor) => Promise<void>;
  connect: (from: number, to: number) => Promise<void>;
  disconnect: (from: number, to: number) => Promise<void>;
  setParam: (id: number, param: "gainDb" | "channel", value: number) => void;
  setBypass: (id: number, bypass: boolean) => Promise<void>;
  openEditor: (id: number) => Promise<void>;
  scan: () => Promise<void>;
  apply: (v: StudioView) => void;
  openSettings: () => void;
  toast: (message: string, kind?: Toast["kind"]) => number;
  loadPreset: (name: string) => Promise<void>;
  /** Previous / next preset in the list (wrapping); quick repeats add up. */
  stepPreset: (dir: 1 | -1) => void;
  /** Resolves to "exists" when the name belongs to another preset and overwrite is false. */
  savePreset: (name: string, overwrite?: boolean) => Promise<"ok" | "exists" | "error">;
  newPreset: () => Promise<void>;
  deletePreset: (name: string) => Promise<void>;
  renamePreset: (from: string, to: string) => Promise<boolean>;
  setMetronome: (patch: Partial<MetronomeSettings>) => void;
  tapTempo: () => void;
  setTuner: (enabled: boolean, referenceA4?: number) => void;
  /** Replace the pedalboard layout (kept locally and mirrored to the engine). */
  updateBoard: (board: unknown) => void;
  setView: (view: View) => void;
  /** The pedalboard is being played full-window (frameless); hide the studio chrome. */
  setStage: (on: boolean) => void;
  /** Scenes: snapshots of every plugin's settings. Adding copies the current sound. */
  addScene: () => Promise<void>;
  selectScene: (index: number) => Promise<void>;
  renameScene: (index: number, name: string) => Promise<boolean>;
  deleteScene: (index: number) => Promise<void>;
}

export type View = "rig" | "board";

export interface Ctx {
  studio: StudioView;
  catalog: Catalog;
  presets: PresetList;
  /** Bumps whenever a whole rig is swapped in, so the canvas can re-fit. */
  layoutKey: number;
  view: View;
  stage: boolean;
  /** This UI is a phone remote-controlling the desktop app (play only). */
  remote: boolean;
  actions: StudioActions;
}


export const StudioContext = createContext<Ctx | null>(null);
export const ActionsContext = createContext<StudioActions | null>(null);
export const CatalogContext = createContext<Catalog | null>(null);
export const AudioContext = createContext<AudioStatus | null>(null);

function need<T>(value: T | null, name: string): T {
  if (value === null) throw new Error(`${name} missing`);
  return value;
}

/** Everything (re-renders on every studio change). */
export const useStudio = () => need(useContext(StudioContext), "StudioContext");
/** The actions only: stable, never re-renders. */
export const useActions = () => need(useContext(ActionsContext), "ActionsContext");
export const useCatalog = () => need(useContext(CatalogContext), "CatalogContext");
/** The running audio device (null when audio is off). */
export const useAudioStatus = () => useContext(AudioContext);

/** Previous / next preset, for the stage HUD and the phone. */
export function usePresetStep() {
  const { presets, actions } = useStudio();
  return { step: actions.stepPreset, count: presets.presets.length, current: presets.current };
}

/** The main output's mute (the Output node's on/off flag). */
export function useOutputMute() {
  const { studio, actions } = useStudio();
  const out = studio.nodes.find((n) => n.kind === "output");
  const muted = !!out?.bypass;
  return { muted, toggle: () => out && actions.setBypass(out.id, !muted) };
}
