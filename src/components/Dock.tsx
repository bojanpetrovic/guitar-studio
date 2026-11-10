// Always-on strip with a compact tuner and metronome. It sits under the canvas in every
// view and floats under the board on stage. It also owns the transport keys:
//   Space      start / stop the metronome
//   ↑ / ↓      ±1 BPM (⇧ ±5)
//   1 – 9      switch scene
// (← / → are left for switching presets on stage.)
import { useEffect, useRef } from "react";
import { Minus, Pause, Play, Plus, Power, Volume2 } from "lucide-react";
import { useStudio } from "../App";
import { useTelemetry } from "../api";

const SUBDIVISIONS = [
  { value: 1, label: "♩" },
  { value: 2, label: "♫" },
  { value: 3, label: "3" },
  { value: 4, label: "16" },
];
const BPM_MIN = 30;
const BPM_MAX = 300;

export function BeatDots({ beats, enabled }: { beats: number; enabled: boolean }) {
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

function DockTuner() {
  const { studio, actions } = useStudio();
  const on = studio.tunerEnabled;
  const a4 = studio.referenceA4;
  const reading = useTelemetry((t) => t?.tuner ?? null);
  const live = on && !!reading;
  const cents = reading?.cents ?? 0;
  const abs = Math.abs(cents);
  const state = !live ? "idle" : abs < 3 ? "in-tune" : abs < 15 ? "close" : "off";
  const changeA4 = (d: number) => actions.setTuner(on, Math.max(430, Math.min(450, a4 + d)));

  return (
    <div className={`dock-tuner ${state} ${on ? "" : "disabled"}`}>
      <button
        className={`icon-btn ghost dock-power ${on ? "on" : ""}`}
        onClick={() => actions.setTuner(!on, a4)}
        title={on ? "Turn the tuner off" : "Turn the tuner on"}
      >
        <Power size={14} />
      </button>
      <span className="dock-note">
        {live ? reading!.note : "–"}
        <small>{live ? reading!.octave : ""}</small>
      </span>
      <span className="dock-strip" aria-hidden>
        {Array.from({ length: 21 }, (_, i) => {
          const c = (i - 10) * 5;
          const lit = live && Math.abs(c - Math.max(-50, Math.min(50, cents))) <= 2.5;
          return <span key={i} className={`dock-tick ${i === 10 ? "center" : ""} ${lit ? "lit" : ""}`} />;
        })}
      </span>
      <span className="dock-cents mono">
        {live ? `${cents > 0 ? "+" : ""}${cents.toFixed(1)}¢` : on ? "play" : "off"}
      </span>
      <span className="dock-hz mono muted">{live ? `${reading!.frequency.toFixed(1)} Hz` : ""}</span>
      <span className="dock-a4 mono" title="Reference pitch">
        <button className="icon-btn ghost mini" onClick={() => changeA4(-1)} aria-label="Lower reference">
          <Minus size={11} />
        </button>
        A4 {a4}
        <button className="icon-btn ghost mini" onClick={() => changeA4(1)} aria-label="Raise reference">
          <Plus size={11} />
        </button>
      </span>
    </div>
  );
}

function DockMetronome() {
  const { studio, actions } = useStudio();
  const m = studio.metronome;
  const set = actions.setMetronome;
  const bpm = Math.round(m.bpm);

  return (
    <div className={`dock-metro ${m.enabled ? "running" : ""}`}>
      <button
        className={`dock-play ${m.enabled ? "on" : ""}`}
        onClick={() => set({ enabled: !m.enabled })}
        title={m.enabled ? "Stop metronome (Space)" : "Start metronome (Space)"}
      >
        {m.enabled ? <Pause size={15} /> : <Play size={15} />}
      </button>
      <BeatDots beats={m.beatsPerBar} enabled={m.enabled} />
      <span className="dock-bpm">
        <button className="icon-btn ghost mini" onClick={() => set({ bpm: bpm - 1 })} aria-label="Slower" title="Slower (↓)">
          <Minus size={12} />
        </button>
        <span className="dock-bpm-value mono" title="Tempo (↑ / ↓, ⇧ for 5)">
          {bpm}
          <small>BPM</small>
        </span>
        <button className="icon-btn ghost mini" onClick={() => set({ bpm: bpm + 1 })} aria-label="Faster" title="Faster (↑)">
          <Plus size={12} />
        </button>
      </span>
      <button className="btn small dock-tap" onClick={actions.tapTempo} title="Tap tempo">
        Tap
      </button>
      <select
        className="dock-select"
        value={m.beatsPerBar}
        onChange={(e) => set({ beatsPerBar: Number(e.target.value) })}
        title="Beats per bar"
      >
        {[2, 3, 4, 5, 6, 7].map((b) => (
          <option key={b} value={b}>
            {b}/4
          </option>
        ))}
      </select>
      <select
        className="dock-select dock-subdiv"
        value={m.subdivision}
        onChange={(e) => set({ subdivision: Number(e.target.value) })}
        title="Subdivision"
      >
        {SUBDIVISIONS.map((s) => (
          <option key={s.value} value={s.value}>
            {s.label}
          </option>
        ))}
      </select>
      <span className="dock-volume" title="Click volume">
        <Volume2 size={14} className="muted" />
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={m.volume}
          style={{ "--pct": `${m.volume * 100}%` } as React.CSSProperties}
          onChange={(e) => set({ volume: Number(e.target.value) })}
        />
      </span>
    </div>
  );
}

/** Space, ↑ / ↓ and 1-9 work from anywhere except text fields. */
function useTransportKeys() {
  const { studio, actions } = useStudio();
  const metro = useRef(studio.metronome);
  metro.current = studio.metronome;
  const sceneCount = useRef(studio.scenes.length);
  sceneCount.current = studio.scenes.length;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Handlers that claim a key (e.g. nudging a selected widget) call preventDefault first.
      if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target?.closest?.("input, textarea, select, [contenteditable='true'], .modal")) return;

      const digit = /^Digit([1-9])$|^Numpad([1-9])$/.exec(e.code);
      if (digit && !e.shiftKey) {
        const index = Number(digit[1] ?? digit[2]) - 1;
        if (index < sceneCount.current) {
          e.preventDefault();
          if (!e.repeat) actions.selectScene(index);
        }
        return;
      }

      const m = metro.current;
      if (e.code === "Space") {
        e.preventDefault();
        // A focused button would otherwise also be "clicked" by the space bar.
        if (target instanceof HTMLButtonElement) target.blur();
        if (e.repeat) return;
        const next = { ...m, enabled: !m.enabled };
        metro.current = next;
        actions.setMetronome({ enabled: next.enabled });
        return;
      }
      const dir = e.key === "ArrowUp" ? 1 : e.key === "ArrowDown" ? -1 : 0;
      if (!dir) return;
      e.preventDefault();
      // Track the value locally so held-down keys don't fall behind React renders.
      const bpm = Math.max(BPM_MIN, Math.min(BPM_MAX, Math.round(m.bpm) + dir * (e.shiftKey ? 5 : 1)));
      metro.current = { ...m, bpm };
      actions.setMetronome({ bpm });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actions]);
}

export default function Dock() {
  useTransportKeys();
  return (
    <footer className="dock">
      <DockTuner />
      <span className="dock-sep" />
      <DockMetronome />
      <span className="dock-hint muted">
        <kbd>Space</kbd> start/stop · <kbd>↑</kbd>
        <kbd>↓</kbd> tempo · <kbd>1</kbd>–<kbd>9</kbd> scenes
      </span>
    </footer>
  );
}
