import { useEffect, useState } from "react";

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  format?: (v: number) => string;
  onChange: (v: number) => void;
  /** Double-click resets to this value. */
  resetTo?: number;
}

/** Horizontal slider with a filled track and a live readout. */
export default function Fader({ label, value, min, max, step = 0.5, unit = "", format, onChange, resetTo }: Props) {
  const [local, setLocal] = useState(value);
  useEffect(() => setLocal(value), [value]);
  const pct = ((local - min) / (max - min)) * 100;
  const set = (v: number) => {
    setLocal(v);
    onChange(v);
  };

  return (
    <label className="fader nodrag" onDoubleClick={() => resetTo !== undefined && set(resetTo)}>
      <span className="fader-head">
        <span className="fader-label">{label}</span>
        <span className="fader-value mono">
          {format ? format(local) : local.toFixed(1)}
          {unit && <small>{unit}</small>}
        </span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={local}
        style={{ "--pct": `${pct}%` } as React.CSSProperties}
        onChange={(e) => set(Number(e.target.value))}
      />
    </label>
  );
}

export const formatDb = (v: number) => (v <= -59.9 ? "-∞" : `${v > 0 ? "+" : ""}${v.toFixed(1)}`);
