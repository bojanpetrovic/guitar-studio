import { useSyncExternalStore } from "react";
import { call as invoke, on } from "./transport";

// ---------------------------------------------------------------- types (mirror Rust)

export type NodeType = "input" | "output" | "gain" | "plugin" | "mixer";

export interface MixerStrip {
  /** Node feeding this strip. */
  source: number;
  gainDb: number;
  /** -1 left … 0 center … +1 right */
  pan: number;
  mute: boolean;
}

export type StripField = "gainDb" | "pan" | "mute";

export interface PluginDescriptor {
  name: string;
  vendor: string;
  version: number;
  category: "effect" | "instrument" | "noteEffect" | "analyzer" | "tool";
  path: string;
  uniqueId: string;
  format: "vst3" | "au" | "au3" | "clap";
  hasEditor: boolean;
  acceptsMidi: boolean;
}

export const pluginKey = (p: PluginDescriptor) => `${p.format}:${p.uniqueId}`;

export interface NodeView {
  id: number;
  kind: NodeType;
  x: number;
  y: number;
  gainDb: number;
  bypass: boolean;
  channel: number;
  plugin: PluginDescriptor | null;
  /** Mixer nodes: one strip per connected input, in cable order. */
  strips: MixerStrip[];
}

export interface AudioConfig {
  inputDevice: string | null;
  outputDevice: string | null;
  bufferSize: number;
}

export interface AudioStatus {
  running: boolean;
  inputDevice: string;
  outputDevice: string;
  inputChannels: number;
  outputChannels: number;
  sampleRate: number;
  bufferSize: number;
  latencyMs: number;
}

export interface MetronomeSettings {
  enabled: boolean;
  bpm: number;
  beatsPerBar: number;
  subdivision: number;
  volume: number;
}

export interface StudioView {
  nodes: NodeView[];
  edges: { from: number; to: number }[];
  audio: AudioStatus | null;
  audioConfig: AudioConfig;
  audioError: string | null;
  metronome: MetronomeSettings;
  tunerEnabled: boolean;
  referenceA4: number;
  /** Pedalboard layout for this rig (null until one is created). */
  pedalboard: unknown;
  /** Scene names in order; number keys 1-9 pick them. */
  scenes: string[];
  activeScene: number | null;
}

export interface DeviceInfo {
  name: string;
  channels: number;
  isDefault: boolean;
}

export interface DeviceList {
  inputs: DeviceInfo[];
  outputs: DeviceInfo[];
}

export interface Catalog {
  plugins: PluginDescriptor[];
  errors: string[];
  scannedAt: number | null;
  scanning: boolean;
}

export interface TunerReading {
  frequency: number;
  note: string;
  octave: number;
  cents: number;
  clarity: number;
}

export interface Telemetry {
  nodes: { id: number; peak: [number, number] }[];
  strips: { mixer: number; source: number; peak: [number, number] }[];
  outputPeak: [number, number];
  cpuLoad: number;
  underruns: number;
  beat: number;
  beatCounter: number;
  tuner: TunerReading | null;
}

export interface ParamInfo {
  index: number;
  id: number;
  name: string;
  unit: string;
  min: number;
  max: number;
  default: number;
  steps: number;
  isBypass: boolean;
}

export interface PresetInfo {
  name: string;
  savedAt: number;
  plugins: string[];
}

export interface PresetList {
  presets: PresetInfo[];
  current: string | null;
  last: string | null;
  dir: string;
}

export interface PresetLoad {
  studio: StudioView;
  presets: PresetList;
  warnings: string[];
  /** Rig generation: bumps on every whole-rig swap. */
  rig: number;
}

/** What the other screen (desktop ↔ phone) pushes as the `sync` event after a change. */
export interface SyncState {
  studio: StudioView;
  preset: string | null;
  rig: number;
}

export interface RemoteStatus {
  enabled: boolean;
  running: boolean;
  /** The link the QR code encodes. */
  url: string | null;
  qrSvg: string | null;
  clients: number;
  error: string | null;
}

/** Error string the backend returns when a save would replace another preset. */
export const PRESET_EXISTS = "PRESET_EXISTS";

// ---------------------------------------------------------------- commands

/** Must match gs_engine::scenes::MAX_SCENES (one per number key). */
export const MAX_SCENES = 9;

export const api = {
  getStudio: () => invoke<StudioView>("get_studio"),
  listDevices: () => invoke<DeviceList>("list_devices"),
  setAudioConfig: (config: AudioConfig) => invoke<StudioView>("set_audio_config", { config }),
  getCatalog: () => invoke<Catalog>("get_catalog"),
  scanPlugins: () => invoke<Catalog>("scan_plugins"),
  addPluginNode: (key: string, x: number, y: number) =>
    invoke<[number, StudioView]>("add_plugin_node", { key, x, y }),
  addGainNode: (x: number, y: number) => invoke<[number, StudioView]>("add_gain_node", { x, y }),
  addMixerNode: (x: number, y: number) => invoke<[number, StudioView]>("add_mixer_node", { x, y }),
  setMixerStrip: (id: number, source: number, field: StripField, value: number) =>
    invoke<void>("set_mixer_strip", { id, source, field, value }),
  removeNode: (id: number) => invoke<StudioView>("remove_node", { id }),
  connect: (from: number, to: number) => invoke<StudioView>("connect", { from, to }),
  disconnect: (from: number, to: number) => invoke<StudioView>("disconnect", { from, to }),
  setNodeParam: (id: number, param: "gainDb" | "channel", value: number) =>
    invoke<void>("set_node_param", { id, param, value }),
  setBypass: (id: number, bypass: boolean) => invoke<void>("set_bypass", { id, bypass }),
  moveNode: (id: number, x: number, y: number) => invoke<void>("move_node", { id, x, y }),
  openEditor: (id: number) => invoke<void>("open_editor", { id }),
  setMetronome: (settings: MetronomeSettings) => invoke<void>("set_metronome", { settings }),
  setTuner: (enabled: boolean, referenceA4?: number) =>
    invoke<void>("set_tuner", { enabled, referenceA4: referenceA4 ?? null }),

  pluginParams: (id: number) => invoke<ParamInfo[]>("plugin_params", { id }),
  setPluginParam: (id: number, paramId: number, value: number) =>
    invoke<void>("set_plugin_param", { id, paramId, value }),
  getParamValues: (items: [number, number][]) => invoke<(number | null)[]>("get_param_values", { items }),
  setPedalboard: (board: unknown) => invoke<void>("set_pedalboard", { board }),
  addScene: (name?: string) => invoke<StudioView>("add_scene", { name: name ?? null }),
  selectScene: (index: number) => invoke<StudioView>("select_scene", { index }),
  renameScene: (index: number, name: string) => invoke<StudioView>("rename_scene", { index, name }),
  deleteScene: (index: number) => invoke<StudioView>("delete_scene", { index }),

  listPresets: () => invoke<PresetList>("list_presets"),
  /** True when loading needs no plugin started (all running or kept warm): instant. */
  presetReady: (name: string) => invoke<boolean>("preset_ready", { name }),
  savePreset: (name: string, overwrite = false) =>
    invoke<[PresetList, string[]]>("save_preset", { name, overwrite }),
  loadPreset: (name: string) => invoke<PresetLoad>("load_preset", { name }),
  newPreset: () => invoke<PresetLoad>("new_preset"),
  deletePreset: (name: string) => invoke<PresetList>("delete_preset", { name }),
  renamePreset: (from: string, to: string) => invoke<PresetList>("rename_preset", { from, to }),

  remoteSync: () => invoke<SyncState>("remote_sync"),
  remoteStatus: () => invoke<RemoteStatus>("remote_status"),
  setRemote: (enabled: boolean) => invoke<RemoteStatus>("set_remote", { enabled }),
  renewRemoteLink: () => invoke<RemoteStatus>("renew_remote_link"),
};

// ---------------------------------------------------------------- telemetry store
// 30 Hz updates go into a tiny external store so only meter components re-render.

let telemetry: Telemetry | null = null;
const subscribers = new Set<() => void>();

on<Telemetry>("telemetry", (t) => {
  telemetry = t;
  subscribers.forEach((fn) => fn());
});

const subscribe = (fn: () => void) => {
  subscribers.add(fn);
  return () => subscribers.delete(fn);
};

export function useTelemetry<T>(select: (t: Telemetry | null) => T): T {
  return useSyncExternalStore(subscribe, () => select(telemetry));
}

/** Linear peak → 0..1 meter position over a -60..0 dB range. */
export function meterLevel(peak: number): number {
  if (peak <= 0.001) return 0;
  const db = 20 * Math.log10(peak);
  return Math.max(0, Math.min(1, (db + 60) / 60));
}

/** Display name for a rig node. */
export function nodeName(nodes: NodeView[], id: number): string {
  const n = nodes.find((x) => x.id === id);
  if (!n) return "Missing";
  if (n.plugin) return n.plugin.name;
  return { input: "Guitar In", output: "Main Out", gain: "Gain", mixer: "Mixer", plugin: "Plugin" }[n.kind];
}

export const formatLabel: Record<PluginDescriptor["format"], string> = {
  vst3: "VST3",
  au: "AU",
  au3: "AUv3",
  clap: "CLAP",
};

/** Let React paint (e.g. a "loading" toast) before a main-thread-blocking command. */
export const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => setTimeout(r, 16)));
