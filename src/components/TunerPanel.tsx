import { Minus, Plus } from "lucide-react";
import { useStudio } from "../App";
import { useTelemetry } from "../api";

const ARC = 50; // degrees of needle swing at ±50 cents

function polar(cx: number, cy: number, r: number, deg: number) {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
}

function Gauge({ cents, active, state }: { cents: number; active: boolean; state: string }) {
  const cx = 140;
  const cy = 150;
  const ticks = [];
  for (let c = -50; c <= 50; c += 5) {
    const major = c % 25 === 0;
    const [x1, y1] = polar(cx, cy, major ? 104 : 110, (c / 50) * ARC);
    const [x2, y2] = polar(cx, cy, 120, (c / 50) * ARC);
    ticks.push(<line key={c} x1={x1} y1={y1} x2={x2} y2={y2} className={`tick ${major ? "major" : ""} ${c === 0 ? "zero" : ""}`} />);
  }
  const angle = active ? (Math.max(-50, Math.min(50, cents)) / 50) * ARC : 0;
  const [ax, ay] = polar(cx, cy, 120, -ARC);
  const [bx, by] = polar(cx, cy, 120, ARC);

  return (
    <svg viewBox="0 0 280 170" className={`gauge ${state}`}>
      <path d={`M ${ax} ${ay} A 120 120 0 0 1 ${bx} ${by}`} className="gauge-arc" />
      {ticks}
      <text x={cx - 112} y={cy - 38} className="gauge-label">♭</text>
      <text x={cx + 104} y={cy - 38} className="gauge-label">♯</text>
      <g className="needle" style={{ transform: `rotate(${angle}deg)`, transformOrigin: `${cx}px ${cy}px` }}>
        <line x1={cx} y1={cy} x2={cx} y2={cy - 116} />
      </g>
      <circle cx={cx} cy={cy} r={7} className="hub" />
    </svg>
  );
}

export default function TunerPanel() {
  const { studio, actions } = useStudio();
  const enabled = studio.tunerEnabled;
  const a4 = studio.referenceA4;
  const reading = useTelemetry((t) => t?.tuner ?? null);

  const toggle = () => actions.setTuner(!enabled, a4);
  const changeA4 = (d: number) => actions.setTuner(enabled, Math.max(430, Math.min(450, a4 + d)));

  const active = enabled && !!reading;
  const cents = reading?.cents ?? 0;
  const abs = Math.abs(cents);
  const state = !active ? "idle" : abs < 3 ? "in-tune" : abs < 15 ? "close" : "off";
  const [letter, accidental] = reading ? [reading.note[0], reading.note.slice(1)] : ["–", ""];

  return (
    <section className={`panel tuner ${enabled ? "" : "disabled"}`}>
      <div className="panel-head">
        <h2>Tuner</h2>
        <button className={`switch ${enabled ? "on" : ""}`} onClick={toggle} aria-label="Toggle tuner">
          <span />
        </button>
      </div>

      <div className={`tuner-display ${state}`}>
        <Gauge cents={cents} active={active} state={state} />
        <div className="note">
          <span className="note-letter">{letter}</span>
          <span className="note-meta">
            <span className="note-acc">{accidental}</span>
            <span className="note-oct">{reading?.octave ?? ""}</span>
          </span>
        </div>
        <div className="tuner-readout mono">
          <span>{active ? `${cents > 0 ? "+" : ""}${cents.toFixed(1)}¢` : enabled ? "play a string" : "off"}</span>
          <span className="muted">{active ? `${reading!.frequency.toFixed(1)} Hz` : ""}</span>
        </div>
      </div>

      <div className="row between">
        <span className="micro-label">Reference</span>
        <div className="stepper">
          <button className="icon-btn ghost" onClick={() => changeA4(-1)} aria-label="Lower reference">
            <Minus size={13} />
          </button>
          <span className="mono">A4 = {a4} Hz</span>
          <button className="icon-btn ghost" onClick={() => changeA4(1)} aria-label="Raise reference">
            <Plus size={13} />
          </button>
        </div>
      </div>
    </section>
  );
}
