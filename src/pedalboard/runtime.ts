import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useStudio } from "../studio";
import { api, nodeName as nameOf, ParamInfo, pluginKey, StudioView } from "../api";
import { clamp01, FLOOR_DB, formatDb, formatPan } from "../util";
import { Board, Widget } from "./types";

/** How long a value set from the board wins over polled values (covers the round trip). */
const OVERRIDE_MS = 700;
const POLL_MS = 150;

const key = (node: number, paramId: number) => `${node}:${paramId}`;

export interface Runtime {
  info: (node: number, paramId: number) => ParamInfo | undefined;
  params: (node: number) => ParamInfo[] | undefined;
  value: (node: number, paramId: number) => number | undefined;
  setParam: (node: number, paramId: number, value: number) => void;
}

/**
 * Plugin-parameter plumbing for the board: fetches parameter lists per plugin node,
 * polls the values of every bound parameter (so knobs follow changes made inside the
 * plugin's own editor), and keeps optimistic values while the user is dragging.
 */
export function useParamRuntime(board: Board, studio: StudioView, active: boolean): Runtime {
  const [paramsByNode, setParamsByNode] = useState<Map<string, ParamInfo[]>>(new Map());
  const [values, setValues] = useState<Map<string, number>>(new Map());
  const overrides = useRef(new Map<string, { v: number; t: number }>());
  const requested = useRef(new Set<string>());
  const [, bump] = useState(0);

  // Cache is keyed by node id + plugin id: a preset load can reuse a node id for a different plugin.
  const pluginKeyOf = useCallback(
    (node: number) => {
      const n = studio.nodes.find((x) => x.id === node);
      return n?.plugin ? `${node}:${pluginKey(n.plugin)}` : undefined;
    },
    [studio.nodes],
  );

  // Fetch each plugin's parameter list once (a failed fetch may retry on the next change).
  useEffect(() => {
    for (const n of studio.nodes) {
      const k = pluginKeyOf(n.id);
      if (!k || requested.current.has(k)) continue;
      requested.current.add(k);
      api.pluginParams(n.id).then(
        (list) => setParamsByNode((m) => new Map(m).set(k, list)),
        () => requested.current.delete(k),
      );
    }
  }, [studio.nodes, pluginKeyOf]);

  // The bound parameters, as a stable signature: moving or restyling widgets doesn't
  // restart the poll, only rebinding does.
  const signature = useMemo(() => {
    const seen = new Set<string>();
    for (const w of board.widgets) if (w.binding.kind === "param") seen.add(key(w.binding.node, w.binding.paramId));
    return [...seen].sort().join(",");
  }, [board.widgets]);

  useEffect(() => {
    if (!active || !signature) return;
    const bound = signature.split(",").map((k) => k.split(":").map(Number) as [number, number]);
    let stop = false;
    let timer: number | undefined;
    // One request at a time (a slow reply, e.g. while a plugin loads, must not be
    // overtaken by a newer one), and no re-render when nothing changed.
    const tick = async () => {
      try {
        const res = await api.getParamValues(bound);
        if (stop) return;
        setValues((prev) => {
          const next = new Map<string, number>();
          res.forEach((v, i) => v !== null && next.set(key(bound[i][0], bound[i][1]), v));
          const same = next.size === prev.size && [...next].every(([k, v]) => prev.get(k) === v);
          return same ? prev : next;
        });
      } catch {
        /* node removed mid-poll, or the link is down; the next tick recovers */
      }
      if (!stop) timer = window.setTimeout(tick, POLL_MS);
    };
    tick();
    return () => {
      stop = true;
      window.clearTimeout(timer);
    };
  }, [active, signature]);

  return useMemo<Runtime>(
    () => ({
      params: (node) => {
        const k = pluginKeyOf(node);
        return k ? paramsByNode.get(k) : undefined;
      },
      info: (node, paramId) => {
        const k = pluginKeyOf(node);
        return k ? paramsByNode.get(k)?.find((p) => p.id === paramId) : undefined;
      },
      value: (node, paramId) => {
        const k = key(node, paramId);
        const o = overrides.current.get(k);
        if (o && performance.now() - o.t < OVERRIDE_MS) return o.v;
        return values.get(k);
      },
      setParam: (node, paramId, value) => {
        overrides.current.set(key(node, paramId), { v: value, t: performance.now() });
        bump((x) => x + 1);
        api.setPluginParam(node, paramId, value).catch(() => {});
      },
    }),
    [paramsByNode, values, pluginKeyOf],
  );
}

export const RuntimeContext = createContext<Runtime | null>(null);
export const useRuntime = () => {
  const r = useContext(RuntimeContext);
  if (!r) throw new Error("RuntimeContext missing");
  return r;
};

// ---------------------------------------------------------------- binding resolution

export interface Resolved {
  /** The target exists (plugin loaded, node present). */
  ok: boolean;
  /** Short description of the target for the UI. */
  target: string;
  // Continuous controls (knob / fader / selector)
  norm: number;
  defaultNorm: number;
  display: string;
  setNorm: (n: number) => void;
  /** Discrete positions (0 = continuous). */
  steps: number;
  // Switches
  on: boolean;
  toggle: () => void;
  press: () => void;
  release: () => void;
}

const noop = () => {};

function fmtNumber(v: number) {
  const a = Math.abs(v);
  return a >= 1000 ? v.toFixed(0) : a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
}

function formatParam(v: number, info: ParamInfo) {
  if (info.unit) return `${fmtNumber(v)} ${info.unit}`;
  if (info.min === 0 && info.max === 1) return `${Math.round(v * 100)}%`;
  return fmtNumber(v);
}

const BASE: Resolved = {
  ok: false,
  target: "Not connected",
  norm: 0,
  defaultNorm: 0,
  display: "",
  setNorm: noop,
  steps: 0,
  on: false,
  toggle: noop,
  press: noop,
  release: noop,
};

export function useResolved(w: Widget): Resolved {
  const { studio, actions } = useStudio();
  const rt = useRuntime();
  const b = w.binding;
  const nodeName = (id: number) => (studio.nodes.some((x) => x.id === id) ? nameOf(studio.nodes, id) : undefined);

  switch (b.kind) {
    case "none":
      return BASE;

    case "param": {
      const info = rt.info(b.node, b.paramId);
      const owner = nodeName(b.node);
      if (!info || !owner) return { ...BASE, target: `${owner ?? "Missing plugin"} · ${b.paramName}` };
      const lo = w.options.min ?? info.min;
      const hi = w.options.max ?? info.max;
      const span = hi - lo || 1;
      const v = rt.value(b.node, b.paramId) ?? info.default;
      const set = (native: number) => rt.setParam(b.node, b.paramId, native);
      const onV = w.options.onValue ?? info.max;
      const offV = w.options.offValue ?? info.min;
      const on = Math.abs(v - onV) < Math.abs(v - offV);
      return {
        ...BASE,
        ok: true,
        target: `${owner} · ${info.name}`,
        norm: clamp01((v - lo) / span),
        defaultNorm: clamp01((info.default - lo) / span),
        display: formatParam(v, info),
        steps: info.steps,
        setNorm: (n) => {
          let native = lo + clamp01(n) * span;
          if (info.steps > 0) {
            const stepSize = (info.max - info.min) / info.steps;
            native = info.min + Math.round((native - info.min) / stepSize) * stepSize;
          }
          set(native);
        },
        on,
        toggle: () => set(on ? offV : onV),
        press: () => set(onV),
        release: () => set(offV),
      };
    }

    case "bypass": {
      const n = studio.nodes.find((x) => x.id === b.node);
      if (!n) return { ...BASE, target: "Missing node" };
      const active = !n.bypass;
      return {
        ...BASE,
        ok: true,
        target: `${nodeName(b.node)} · on/off`,
        display: active ? "ON" : "OFF",
        on: active,
        toggle: () => actions.setBypass(n.id, !n.bypass),
        press: () => actions.setBypass(n.id, false),
        release: () => actions.setBypass(n.id, true),
      };
    }

    case "level": {
      const n = studio.nodes.find((x) => x.id === b.node);
      if (!n) return { ...BASE, target: "Missing node" };
      // Same ranges as the rig's faders: input trim ±24 dB, master up to +12.
      const lo = w.options.min ?? (n.kind === "input" ? -24 : FLOOR_DB);
      const hi = w.options.max ?? (n.kind === "output" ? 12 : 24);
      return {
        ...BASE,
        ok: true,
        target: `${nodeName(b.node)} · level`,
        norm: clamp01((n.gainDb - lo) / (hi - lo)),
        defaultNorm: clamp01((0 - lo) / (hi - lo)),
        display: `${formatDb(n.gainDb)} dB`,
        setNorm: (x) => actions.setParam(n.id, "gainDb", Math.round((lo + clamp01(x) * (hi - lo)) * 10) / 10),
      };
    }

    case "mix": {
      const mixer = studio.nodes.find((x) => x.id === b.node && x.kind === "mixer");
      const strip = mixer?.strips.find((st) => st.source === b.source);
      const label = `${nodeName(b.source) ?? "Missing input"} (${mixer ? "Mixer" : "missing mixer"})`;
      if (!mixer || !strip) return { ...BASE, target: label };
      const set = (field: "gainDb" | "pan" | "mute", v: number) => actions.setMixerStrip(mixer.id, b.source, field, v);
      if (b.field === "mute") {
        const on = !strip.mute;
        return {
          ...BASE,
          ok: true,
          target: `${label} · on/off`,
          display: on ? "ON" : "MUTED",
          on,
          toggle: () => set("mute", on ? 1 : 0),
          press: () => set("mute", 0),
          release: () => set("mute", 1),
        };
      }
      if (b.field === "pan") {
        const lo = w.options.min ?? -1;
        const hi = w.options.max ?? 1;
        const p = strip.pan;
        return {
          ...BASE,
          ok: true,
          target: `${label} · pan`,
          norm: clamp01((p - lo) / (hi - lo)),
          defaultNorm: clamp01((0 - lo) / (hi - lo)),
          display: formatPan(p),
          setNorm: (x) => {
            const v = lo + clamp01(x) * (hi - lo);
            set("pan", Math.abs(v) < 0.03 ? 0 : Math.round(v * 100) / 100);
          },
        };
      }
      const lo = w.options.min ?? FLOOR_DB;
      const hi = w.options.max ?? 12;
      return {
        ...BASE,
        ok: true,
        target: `${label} · volume`,
        norm: clamp01((strip.gainDb - lo) / (hi - lo)),
        defaultNorm: clamp01((0 - lo) / (hi - lo)),
        display: `${formatDb(strip.gainDb)} dB`,
        setNorm: (x) => set("gainDb", Math.round((lo + clamp01(x) * (hi - lo)) * 10) / 10),
      };
    }

    case "bpm": {
      const lo = w.options.min ?? 40;
      const hi = w.options.max ?? 240;
      const bpm = studio.metronome.bpm;
      return {
        ...BASE,
        ok: true,
        target: "Tempo",
        norm: clamp01((bpm - lo) / (hi - lo)),
        defaultNorm: clamp01((120 - lo) / (hi - lo)),
        display: `${Math.round(bpm)} BPM`,
        setNorm: (x) => actions.setMetronome({ bpm: lo + clamp01(x) * (hi - lo) }),
      };
    }

    case "metronome": {
      const on = studio.metronome.enabled;
      const set = (enabled: boolean) => actions.setMetronome({ enabled });
      return {
        ...BASE,
        ok: true,
        target: "Metronome",
        display: `${Math.round(studio.metronome.bpm)} BPM`,
        on,
        toggle: () => set(!on),
        press: () => set(true),
        release: () => set(false),
      };
    }

    case "tuner": {
      const on = studio.tunerEnabled;
      return {
        ...BASE,
        ok: true,
        target: "Tuner",
        display: on ? "TUNER" : "",
        on,
        toggle: () => actions.setTuner(!on),
        press: () => actions.setTuner(true),
        release: () => actions.setTuner(false),
      };
    }

    case "tap":
      return {
        ...BASE,
        ok: true,
        target: "Tap tempo",
        display: `${Math.round(studio.metronome.bpm)} BPM`,
        toggle: actions.tapTempo,
        press: actions.tapTempo,
      };

    case "meter":
      return { ...BASE, ok: b.node === "master" || !!nodeName(b.node), target: b.node === "master" ? "Main output" : `${nodeName(b.node) ?? "Missing node"}` };
  }
}
