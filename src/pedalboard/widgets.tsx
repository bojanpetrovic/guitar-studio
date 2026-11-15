import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTelemetry } from "../api";
import { usePeakLevels } from "../hooks";
import { useStudio } from "../studio";
import { clamp01, formatCents, isTyping, tickLit, TUNER_TICKS, tuneState } from "../util";
import { Resolved, useResolved } from "./runtime";
import { keyLabel, PedalFinish, Widget } from "./types";

interface Props {
  w: Widget;
  /** Edit mode: render only, no interaction (the editor overlay handles pointers). */
  editing: boolean;
}

/** Vertical-drag gesture for knobs: 180 px for the full range, Shift for fine control. */
function useDrag(r: Resolved, editing: boolean) {
  const start = useRef<{ y: number; n: number } | null>(null);
  return {
    onPointerDown: (e: React.PointerEvent) => {
      if (editing || !r.ok) return;
      (e.target as Element).setPointerCapture(e.pointerId);
      start.current = { y: e.clientY, n: r.norm };
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (!start.current) return;
      const range = e.shiftKey ? 720 : 180;
      r.setNorm(clamp01(start.current.n + (start.current.y - e.clientY) / range));
    },
    onPointerUp: () => (start.current = null),
    onPointerCancel: () => (start.current = null),
    onWheel: (e: React.WheelEvent) => {
      if (editing || !r.ok) return;
      r.setNorm(clamp01(r.norm - e.deltaY / (e.shiftKey ? 4000 : 1000)));
    },
    onDoubleClick: () => !editing && r.ok && r.setNorm(r.defaultNorm),
  };
}

// ---------------------------------------------------------------- keyboard

type KeyMode =
  | { kind: "switch"; momentary: boolean }
  /** Knob / fader / selector: its key selects it, then ↑ / ↓ adjust it. */
  | { kind: "dial"; positions?: number };

/** Nudge per ↑ / ↓ press for continuous controls (⇧ for coarse). */
const KEY_STEP = 0.01;
const KEY_STEP_COARSE = 0.05;

// The selected key: its knobs / faders / selectors take ↑ / ↓ until the key is
// pressed again, another control's key is pressed, or Esc. It's the key that's
// selected, not one control, so controls sharing a key move together.
let selectedKey: string | null = null;
let lastToggle: Event | null = null;
const selectionListeners = new Set<() => void>();
function selectKey(code: string | null) {
  if (selectedKey === code) return;
  selectedKey = code;
  selectionListeners.forEach((l) => l());
}
/** Every control with the key sees the same keydown: toggle once per event. */
function toggleKey(code: string, e: Event) {
  if (lastToggle === e) return;
  lastToggle = e;
  selectKey(selectedKey === code ? null : code);
}
/** Drop the selection (leaving play mode, another rig). */
export const clearKeySelection = () => selectKey(null);
const subscribeSelection = (l: () => void) => {
  selectionListeners.add(l);
  return () => void selectionListeners.delete(l);
};
const useKeySelected = (code: string | undefined) =>
  useSyncExternalStore(subscribeSelection, () => !!code && selectedKey === code);

/**
 * Plays a control from its assigned key while the board is in play mode. Listens on
 * `document` so a selected control claims ↑ / ↓ (and Esc) before the window-level
 * tempo and stage keys (they skip events that are already handled). Returns whether
 * the control is active: a switch's key held down, or a dial's key selected.
 */
function useControlKey(w: Widget, r: Resolved, editing: boolean, mode: KeyMode) {
  const code = w.options.key;
  const latest = useRef(r);
  latest.current = r;
  const [held, setHeld] = useState(false);
  const selected = useKeySelected(mode.kind === "dial" && !editing ? code : undefined);
  const momentary = mode.kind === "switch" && mode.momentary;
  const positions = mode.kind === "dial" ? (mode.positions ?? 0) : 0;
  const kind = mode.kind;

  useEffect(() => {
    if (!code || editing) return;
    let down = false;
    // Where ↑ / ↓ last put the value: repeats build on it rather than on a value React
    // hasn't rendered yet. Re-read from the control once the keys have been idle.
    let norm = 0;
    let nudgedAt = -Infinity;

    const letGo = () => {
      if (!down) return;
      down = false;
      setHeld(false);
      if (kind === "switch" && momentary && latest.current.ok) latest.current.release();
    };
    const onDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e)) return;
      const r = latest.current;
      if (e.code === code) {
        e.preventDefault();
        if (e.repeat) return;
        if (kind === "dial") {
          toggleKey(code, e);
          return;
        }
        if (down) return;
        down = true;
        setHeld(true);
        if (r.ok) {
          if (momentary) r.press();
          else r.toggle();
        }
        return;
      }
      if (kind !== "dial" || selectedKey !== code) return;
      if (e.key === "Escape") {
        e.preventDefault();
        selectKey(null);
        return;
      }
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      e.preventDefault();
      if (!r.ok) return;
      const now = performance.now();
      if (now - nudgedAt > 400) norm = r.norm;
      nudgedAt = now;
      const discrete = positions > 1 ? positions - 1 : r.steps;
      const step = discrete > 0 ? 1 / discrete : e.shiftKey ? KEY_STEP_COARSE : KEY_STEP;
      norm = clamp01(norm + (e.key === "ArrowUp" ? step : -step));
      r.setNorm(norm);
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code === code) letGo();
    };
    document.addEventListener("keydown", onDown);
    document.addEventListener("keyup", onUp);
    window.addEventListener("blur", letGo);
    return () => {
      document.removeEventListener("keydown", onDown);
      document.removeEventListener("keyup", onUp);
      window.removeEventListener("blur", letGo);
      letGo();
    };
  }, [code, editing, kind, momentary, positions]);

  return held || selected;
}

/** The little key badge in a control's corner. */
function KeyCap({ code, held }: { code?: string; held: boolean }) {
  if (!code) return null;
  return <span className={`pb-keycap mono ${held ? "held" : ""}`}>{keyLabel(code)}</span>;
}

// ---------------------------------------------------------------- footswitch

function Footswitch({ w, editing }: Props) {
  const r = useResolved(w);
  const isTap = w.binding.kind === "tap";
  // Only tap switches pulse with the beat; the others needn't re-render on every beat.
  const beat = useTelemetry((t) => (isTap ? (t?.beatCounter ?? 0) : 0));
  const { studio } = useStudio();
  // A tap-tempo LED pulses on every metronome beat (re-keyed so the animation replays).
  const pulsing = isTap && studio.metronome.enabled;

  const down = (e: React.PointerEvent) => {
    if (editing || !r.ok) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    if (w.options.momentary && !isTap) r.press();
    else r.toggle();
  };
  const up = () => {
    if (!editing && r.ok && w.options.momentary && !isTap) r.release();
  };
  const held = useControlKey(w, r, editing, { kind: "switch", momentary: !!w.options.momentary && !isTap });
  const size = Math.min(w.w * 0.62, w.h * 0.46);

  return (
    <div
      className={`pb-footswitch ${r.ok ? "" : "unbound"} ${held ? "key-held" : ""}`}
      style={{ "--accent": w.color } as React.CSSProperties}
    >
      <KeyCap code={w.options.key} held={held} />
      <span key={pulsing ? beat : "led"} className={`pb-led ${!isTap && r.on ? "on" : ""} ${pulsing ? "pulse" : ""}`} />
      <button
        className="pb-stomp"
        style={{ width: size, height: size }}
        onPointerDown={down}
        onPointerUp={up}
        // A cancelled touch (scroll, OS gesture) must not leave a momentary switch held.
        onPointerCancel={up}
        aria-pressed={r.on}
        aria-label={w.label}
      />
      <span className="pb-caption">{w.label}</span>
      {(isTap || w.binding.kind === "metronome") && <span className="pb-sub mono">{r.display}</span>}
    </div>
  );
}

// ---------------------------------------------------------------- knob

const KNOB_SWEEP = 270;

function arcPath(cx: number, cy: number, r: number, a0: number, a1: number) {
  const p = (a: number) => {
    const rad = ((a - 90) * Math.PI) / 180;
    return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
  };
  const [x0, y0] = p(a0);
  const [x1, y1] = p(a1);
  const large = a1 - a0 > 180 ? 1 : 0;
  return `M ${x0} ${y0} A ${r} ${r} 0 ${large} 1 ${x1} ${y1}`;
}

function Knob({ w, editing }: Props) {
  const r = useResolved(w);
  const drag = useDrag(r, editing);
  const held = useControlKey(w, r, editing, { kind: "dial" });
  const size = Math.max(40, Math.min(w.w - 8, w.h - 44));
  const c = size / 2;
  const rad = c - 5;
  const a0 = -KNOB_SWEEP / 2;
  const a = a0 + r.norm * KNOB_SWEEP;

  return (
    <div className={`pb-knob ${r.ok ? "" : "unbound"} ${held ? "key-held" : ""}`} style={{ "--accent": w.color } as React.CSSProperties}>
      <KeyCap code={w.options.key} held={held} />
      <svg width={size} height={size} className="pb-knob-svg" {...drag}>
        <path d={arcPath(c, c, rad, a0, a0 + KNOB_SWEEP)} className="pb-arc-track" />
        {r.ok && r.norm > 0.001 && <path d={arcPath(c, c, rad, a0, a)} className="pb-arc-value" />}
        <circle cx={c} cy={c} r={rad - 7} className="pb-knob-cap" />
        <g style={{ transform: `rotate(${a}deg)`, transformOrigin: `${c}px ${c}px` }}>
          <line x1={c} y1={c - rad + 10} x2={c} y2={c - rad * 0.35} className="pb-knob-pointer" />
        </g>
      </svg>
      <span className="pb-caption">{w.label}</span>
      <span className="pb-sub mono">{r.ok ? r.display : "—"}</span>
    </div>
  );
}

// ---------------------------------------------------------------- fader

function Fader({ w, editing }: Props) {
  const r = useResolved(w);
  const held = useControlKey(w, r, editing, { kind: "dial" });
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const setFromY = (clientY: number) => {
    const box = track.current?.getBoundingClientRect();
    if (box) r.setNorm(clamp01(1 - (clientY - box.top) / box.height));
  };

  return (
    <div className={`pb-fader ${r.ok ? "" : "unbound"} ${held ? "key-held" : ""}`} style={{ "--accent": w.color } as React.CSSProperties}>
      <KeyCap code={w.options.key} held={held} />
      <div
        ref={track}
        className="pb-fader-track"
        onPointerDown={(e) => {
          if (editing || !r.ok) return;
          (e.target as Element).setPointerCapture(e.pointerId);
          dragging.current = true;
          setFromY(e.clientY);
        }}
        onPointerMove={(e) => dragging.current && setFromY(e.clientY)}
        onPointerUp={() => (dragging.current = false)}
        onPointerCancel={() => (dragging.current = false)}
        onDoubleClick={() => !editing && r.ok && r.setNorm(r.defaultNorm)}
      >
        <div className="pb-fader-fill" style={{ height: `${r.norm * 100}%` }} />
        <div className="pb-fader-cap" style={{ bottom: `${r.norm * 100}%` }} />
      </div>
      <span className="pb-caption">{w.label}</span>
      <span className="pb-sub mono">{r.ok ? r.display : "—"}</span>
    </div>
  );
}

// ---------------------------------------------------------------- selector

function Selector({ w, editing }: Props) {
  const r = useResolved(w);
  const custom = w.options.labels?.filter(Boolean) ?? [];
  const labels =
    custom.length >= 2
      ? custom
      : r.steps > 0 && r.steps <= 11
        ? Array.from({ length: r.steps + 1 }, (_, i) => String(i + 1))
        : ["A", "B", "C"];
  const n = labels.length;
  const active = Math.round(r.norm * (n - 1));
  const held = useControlKey(w, r, editing, { kind: "dial", positions: n });

  return (
    <div className={`pb-selector ${r.ok ? "" : "unbound"} ${held ? "key-held" : ""}`} style={{ "--accent": w.color } as React.CSSProperties}>
      <KeyCap code={w.options.key} held={held} />
      <span className="pb-caption">{w.label}</span>
      <div className="pb-sel-list">
        {labels.map((l, i) => (
          <button
            key={i}
            className={`pb-sel-item ${i === active && r.ok ? "on" : ""}`}
            onClick={() => !editing && r.ok && r.setNorm(n === 1 ? 0 : i / (n - 1))}
          >
            <span className="pb-sel-dot" />
            <span>{l}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- meter

function MeterWidget({ w }: Props) {
  const node = w.binding.kind === "meter" ? w.binding.node : "master";
  const { levels } = usePeakLevels((t) => (node === "master" ? t?.outputPeak : t?.nodes.find((n) => n.id === node)?.peak));
  const segments = Math.max(8, Math.floor((w.h - 30) / 7));

  return (
    <div className="pb-meter">
      <div className="pb-meter-bars">
        {levels.map((lv, c) => (
          <div className="pb-meter-col" key={c}>
            {Array.from({ length: segments }, (_, i) => {
              const pos = (segments - i) / segments;
              const zone = pos > 0.92 ? "red" : pos > 0.75 ? "amber" : "green";
              return <span key={i} className={`pb-seg ${zone} ${lv >= pos - 0.5 / segments ? "lit" : ""}`} />;
            })}
          </div>
        ))}
      </div>
      {w.label && <span className="pb-caption">{w.label}</span>}
    </div>
  );
}

// ---------------------------------------------------------------- label, panel, tuner

function Label({ w }: Props) {
  return (
    <div className="pb-label" style={{ fontSize: w.options.fontSize ?? 22, color: w.color }}>
      {w.label}
    </div>
  );
}

/** A pedal enclosure. Wired to a plugin's on/off, its LED follows it, and an animated
 *  finish runs only while the plugin is on. */
function Panel({ w }: Props) {
  const r = useResolved(w);
  const led = w.binding.kind === "none" ? undefined : r.on;
  return <Pedal finish={w.options.finish} color={w.color} label={w.label} led={led} />;
}

/** The scope finish's traces: periodic over the path, so sliding by half loops. */
const wave = (period: number, amp: number, harmonic: number) => {
  let d = "";
  for (let x = 0; x <= 400; x += 4) {
    const t = (2 * Math.PI * x) / period;
    d += `${x ? "L" : "M"}${x},${(50 + amp * Math.sin(t) + harmonic * Math.sin(2 * t + 1)).toFixed(1)}`;
  }
  return d;
};
const TRACE_A = wave(100, 26, 9);
const TRACE_B = wave(200, 16, 0);

/** A stompbox body: jacks, rim, screws, LED (`undefined`: always lit, it follows
 *  nothing) and title, in one of the PEDAL_FINISHES. Also the inspector's previews. */
export function Pedal({ finish = "classic", color, label, led }: { finish?: PedalFinish; color: string; label: string; led?: boolean }) {
  const off = led === false;
  return (
    <div className={`pb-panel finish-${finish} ${off ? "off" : ""}`} style={{ "--enclosure": color } as React.CSSProperties}>
      {finish === "neon" && <span className="pb-pedal-glow" />}
      <span className="pb-pedal-jack in" />
      <span className="pb-pedal-jack out" />
      <span className="pb-pedal-dc" />
      <div className="pb-pedal-body">
        <span className="pb-pedal-art">
          {finish === "scope" && (
            <>
              <svg className="pb-trace b" viewBox="0 0 400 100" preserveAspectRatio="none">
                <path d={TRACE_B} />
              </svg>
              <svg className="pb-trace a" viewBox="0 0 400 100" preserveAspectRatio="none">
                <path className="halo" d={TRACE_A} />
                <path d={TRACE_A} />
              </svg>
            </>
          )}
          {finish === "lava" && [0, 1, 2, 3].map((i) => <span key={i} className={`pb-blob b${i}`} />)}
        </span>
        <span className="pb-screw tl" />
        <span className="pb-screw tr" />
        <span className="pb-screw bl" />
        <span className="pb-screw br" />
        <span className={`pb-pedal-led ${off ? "" : "on"}`} />
        <span className="pb-panel-title">{label}</span>
      </div>
    </div>
  );
}

function TunerWidget({ w, editing }: Props) {
  const { studio, actions } = useStudio();
  const reading = useTelemetry((t) => t?.tuner ?? null);
  const on = studio.tunerEnabled;
  const cents = reading?.cents ?? 0;
  const state = tuneState(on && reading ? cents : null);

  return (
    <button
      className={`pb-tuner ${state} ${on ? "on" : ""}`}
      style={{ "--accent": w.color } as React.CSSProperties}
      onClick={() => !editing && actions.setTuner(!on)}
      title={on ? "Tuner on — click to turn off" : "Click to turn the tuner on"}
    >
      <span className="pb-tuner-note">
        {on && reading ? reading.note : on ? "–" : "OFF"}
        {on && reading && <small>{reading.octave}</small>}
      </span>
      <span className="pb-tuner-strip">
        {TUNER_TICKS.map((i) => (
          <span key={i} className={`pb-tick ${i === 10 ? "center" : ""} ${on && reading && tickLit(i, cents) ? "lit" : ""}`} />
        ))}
      </span>
      <span className="pb-sub mono">{on && reading ? formatCents(cents) : w.label}</span>
    </button>
  );
}

export function WidgetView(props: Props) {
  switch (props.w.type) {
    case "footswitch":
      return <Footswitch {...props} />;
    case "knob":
      return <Knob {...props} />;
    case "fader":
      return <Fader {...props} />;
    case "selector":
      return <Selector {...props} />;
    case "meter":
      return <MeterWidget {...props} />;
    case "label":
      return <Label {...props} />;
    case "panel":
      return <Panel {...props} />;
    case "tuner":
      return <TunerWidget {...props} />;
  }
}
