import { useRef } from "react";
import { meterLevel, useTelemetry } from "../api";
import { useStudio } from "../App";
import { Resolved, useResolved } from "./runtime";
import { Widget } from "./types";

interface Props {
  w: Widget;
  /** Edit mode: render only, no interaction (the editor overlay handles pointers). */
  editing: boolean;
}

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

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
    onWheel: (e: React.WheelEvent) => {
      if (editing || !r.ok) return;
      r.setNorm(clamp01(r.norm - e.deltaY / (e.shiftKey ? 4000 : 1000)));
    },
    onDoubleClick: () => !editing && r.ok && r.setNorm(r.defaultNorm),
  };
}

// ---------------------------------------------------------------- footswitch

function Footswitch({ w, editing }: Props) {
  const r = useResolved(w);
  const isTap = w.binding.kind === "tap";
  const beat = useTelemetry((t) => t?.beatCounter ?? 0);
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
  const size = Math.min(w.w * 0.62, w.h * 0.46);

  return (
    <div className={`pb-footswitch ${r.ok ? "" : "unbound"}`} style={{ "--accent": w.color } as React.CSSProperties}>
      <span key={pulsing ? beat : "led"} className={`pb-led ${!isTap && r.on ? "on" : ""} ${pulsing ? "pulse" : ""}`} />
      <button
        className="pb-stomp"
        style={{ width: size, height: size }}
        onPointerDown={down}
        onPointerUp={up}
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
  const size = Math.max(40, Math.min(w.w - 8, w.h - 44));
  const c = size / 2;
  const rad = c - 5;
  const a0 = -KNOB_SWEEP / 2;
  const a = a0 + r.norm * KNOB_SWEEP;

  return (
    <div className={`pb-knob ${r.ok ? "" : "unbound"}`} style={{ "--accent": w.color } as React.CSSProperties}>
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
  const track = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const setFromY = (clientY: number) => {
    const box = track.current?.getBoundingClientRect();
    if (box) r.setNorm(clamp01(1 - (clientY - box.top) / box.height));
  };

  return (
    <div className={`pb-fader ${r.ok ? "" : "unbound"}`} style={{ "--accent": w.color } as React.CSSProperties}>
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

  return (
    <div className={`pb-selector ${r.ok ? "" : "unbound"}`} style={{ "--accent": w.color } as React.CSSProperties}>
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

const EMPTY: [number, number] = [0, 0];

function MeterWidget({ w }: Props) {
  const node = w.binding.kind === "meter" ? w.binding.node : "master";
  const peak = useTelemetry((t) =>
    !t ? EMPTY : node === "master" ? t.outputPeak : (t.nodes.find((n) => n.id === node)?.peak ?? EMPTY),
  );
  const shown = useRef<[number, number]>([0, 0]);
  const levels = [0, 1].map((c) => Math.max(meterLevel(peak[c]), shown.current[c] - 0.035)) as [number, number];
  shown.current = levels;
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

function Panel({ w }: Props) {
  return (
    <div className="pb-panel" style={{ "--enclosure": w.color } as React.CSSProperties}>
      <span className="pb-screw tl" />
      <span className="pb-screw tr" />
      <span className="pb-screw bl" />
      <span className="pb-screw br" />
      <span className="pb-panel-title">{w.label}</span>
    </div>
  );
}

function TunerWidget({ w, editing }: Props) {
  const { studio, actions } = useStudio();
  const reading = useTelemetry((t) => t?.tuner ?? null);
  const on = studio.tunerEnabled;
  const cents = reading?.cents ?? 0;
  const state = !on || !reading ? "idle" : Math.abs(cents) < 3 ? "in-tune" : Math.abs(cents) < 15 ? "close" : "off";

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
        {Array.from({ length: 21 }, (_, i) => {
          const c = (i - 10) * 5;
          const lit = on && reading && Math.abs(c - cents) <= 2.5;
          return <span key={i} className={`pb-tick ${i === 10 ? "center" : ""} ${lit ? "lit" : ""}`} />;
        })}
      </span>
      <span className="pb-sub mono">{on && reading ? `${cents > 0 ? "+" : ""}${cents.toFixed(1)}¢` : w.label}</span>
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
