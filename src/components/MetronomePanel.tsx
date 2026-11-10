import { useEffect, useRef, useState } from "react";
import { Minus, Pause, Play, Plus, Volume2 } from "lucide-react";
import { useStudio } from "../App";
import { api, MetronomeSettings, useTelemetry } from "../api";

const SUBDIVISIONS = [
  { value: 1, label: "♩" },
  { value: 2, label: "♫" },
  { value: 3, label: "3" },
  { value: 4, label: "16" },
];

function BeatDots({ beats, enabled }: { beats: number; enabled: boolean }) {
  const beat = useTelemetry((t) => t?.beat ?? 0);
  const counter = useTelemetry((t) => t?.beatCounter ?? 0);
  return (
    <div className="beat-dots">
      {Array.from({ length: beats }, (_, i) => (
        <span
          // Re-key the active dot on every beat so its flash animation restarts.
          key={enabled && i === beat ? `on-${counter}` : `off-${i}`}
          className={`beat-dot ${i === 0 ? "accent" : ""} ${enabled && i === beat ? "active" : ""}`}
        />
      ))}
    </div>
  );
}

export default function MetronomePanel() {
  const { studio, actions } = useStudio();
  const [m, setM] = useState<MetronomeSettings>(studio.metronome);
  const taps = useRef<number[]>([]);

  useEffect(() => {
    api.setMetronome(m).catch((e) => actions.toast(String(e), "error"));
  }, [m, actions]);

  const update = (patch: Partial<MetronomeSettings>) => setM((s) => ({ ...s, ...patch }));
  const setBpm = (bpm: number) => update({ bpm: Math.round(Math.max(30, Math.min(300, bpm))) });

  const tap = () => {
    const now = performance.now();
    taps.current = [...taps.current.filter((t) => now - t < 2500), now].slice(-6);
    if (taps.current.length >= 2) {
      const gaps = taps.current.slice(1).map((t, i) => t - taps.current[i]);
      setBpm(60000 / (gaps.reduce((a, b) => a + b, 0) / gaps.length));
    }
  };

  return (
    <section className="panel metronome">
      <div className="panel-head">
        <h2>Metronome</h2>
        <BeatDots beats={m.beatsPerBar} enabled={m.enabled} />
      </div>

      <div className="bpm-row">
        <button className="icon-btn round" onClick={() => setBpm(m.bpm - 1)} aria-label="Slower">
          <Minus size={16} />
        </button>
        <div className="bpm">
          <span className="bpm-value mono">{Math.round(m.bpm)}</span>
          <span className="micro-label">BPM</span>
        </div>
        <button className="icon-btn round" onClick={() => setBpm(m.bpm + 1)} aria-label="Faster">
          <Plus size={16} />
        </button>
      </div>

      <input
        className="bpm-slider"
        type="range"
        min={30}
        max={300}
        value={m.bpm}
        style={{ "--pct": `${((m.bpm - 30) / 270) * 100}%` } as React.CSSProperties}
        onChange={(e) => setBpm(Number(e.target.value))}
      />

      <div className="row gap">
        <div className="segmented grow">
          {[2, 3, 4, 5, 6, 7].map((b) => (
            <button key={b} className={m.beatsPerBar === b ? "on" : ""} onClick={() => update({ beatsPerBar: b })}>
              {b}/4
            </button>
          ))}
        </div>
      </div>

      <div className="row gap">
        <div className="segmented grow">
          {SUBDIVISIONS.map((s) => (
            <button key={s.value} className={m.subdivision === s.value ? "on" : ""} onClick={() => update({ subdivision: s.value })}>
              {s.label}
            </button>
          ))}
        </div>
        <button className="btn" onClick={tap}>
          Tap
        </button>
      </div>

      <div className="row gap center">
        <Volume2 size={15} className="muted" />
        <input
          className="mini-slider"
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={m.volume}
          style={{ "--pct": `${m.volume * 100}%` } as React.CSSProperties}
          onChange={(e) => update({ volume: Number(e.target.value) })}
        />
      </div>

      <button className={`btn play ${m.enabled ? "on" : ""}`} onClick={() => update({ enabled: !m.enabled })}>
        {m.enabled ? <Pause size={18} /> : <Play size={18} />}
        {m.enabled ? "Stop" : "Start"}
      </button>
    </section>
  );
}
