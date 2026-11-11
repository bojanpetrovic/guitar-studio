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

/** Horizontal slider with a filled track and a live readout. Fully controlled: the
 *  parent updates `value` optimistically. */
export default function Fader({ label, value, min, max, step = 0.5, unit = "", format, onChange, resetTo }: Props) {
  const pct = ((value - min) / (max - min)) * 100;
  const set = onChange;

  return (
    <label className="fader nodrag" onDoubleClick={() => resetTo !== undefined && set(resetTo)}>
      <span className="fader-head">
        <span className="fader-label">{label}</span>
        <span className="fader-value mono">
          {format ? format(value) : value.toFixed(1)}
          {unit && <small>{unit}</small>}
        </span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        style={{ "--pct": `${pct}%` } as React.CSSProperties}
        onChange={(e) => set(Number(e.target.value))}
      />
    </label>
  );
}
