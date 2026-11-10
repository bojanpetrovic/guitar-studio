import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useStudio } from "../App";
import { api, ParamInfo, StudioView } from "../api";
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
  const [, bump] = useState(0);

  // Cache is keyed by node id + plugin id: a preset load can reuse a node id for a different plugin.
  const pluginKeyOf = useCallback(
    (node: number) => {
      const n = studio.nodes.find((x) => x.id === node);
      return n?.plugin ? `${node}:${n.plugin.format}:${n.plugin.uniqueId}` : undefined;
    },
    [studio.nodes],
  );

  useEffect(() => {
    for (const n of studio.nodes) {
      const k = pluginKeyOf(n.id);
      if (!k || paramsByNode.has(k)) continue;
      api
        .pluginParams(n.id)
        .then((list) => setParamsByNode((m) => new Map(m).set(k, list)))
        .catch(() => setParamsByNode((m) => new Map(m).set(k, [])));
    }
  }, [studio.nodes, pluginKeyOf, paramsByNode]);

  const bound = useMemo(() => {
    const seen = new Set<string>();
    const items: [number, number][] = [];
    for (const w of board.widgets) {
      if (w.binding.kind !== "param") continue;
      const k = key(w.binding.node, w.binding.paramId);
      if (!seen.has(k)) {
        seen.add(k);
        items.push([w.binding.node, w.binding.paramId]);
      }
    }
    return items;
  }, [board.widgets]);

  useEffect(() => {
    if (!active || bound.length === 0) return;
    let stop = false;
    const tick = async () => {
      try {
        const res = await api.getParamValues(bound);
        if (stop) return;
        const next = new Map<string, number>();
        res.forEach((v, i) => v !== null && next.set(key(bound[i][0], bound[i][1]), v));
        setValues(next);
      } catch {
        /* node removed mid-poll; next tick recovers */
      }
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      stop = true;
      clearInterval(id);
    };
  }, [active, bound]);

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

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));
const noop = () => {};

function fmtNumber(v: number) {
  const a = Math.abs(v);
  return a >= 1000 ? v.toFixed(0) : a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
}

export function formatParam(v: number, info: ParamInfo) {
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
  const nodeName = (id: number) => {
    const n = studio.nodes.find((x) => x.id === id);
    if (!n) return undefined;
    return n.plugin?.name ?? { input: "Guitar In", output: "Main Out", gain: "Gain", plugin: "Plugin" }[n.kind];
  };

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
      const lo = w.options.min ?? -60;
      const hi = w.options.max ?? (n.kind === "output" ? 12 : n.kind === "input" ? 24 : 24);
      return {
        ...BASE,
        ok: true,
        target: `${nodeName(b.node)} · level`,
        norm: clamp01((n.gainDb - lo) / (hi - lo)),
        defaultNorm: clamp01((0 - lo) / (hi - lo)),
        display: n.gainDb <= -59.9 ? "-∞ dB" : `${n.gainDb > 0 ? "+" : ""}${n.gainDb.toFixed(1)} dB`,
        setNorm: (x) => actions.setParam(n.id, "gainDb", Math.round((lo + clamp01(x) * (hi - lo)) * 10) / 10),
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
