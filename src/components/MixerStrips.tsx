import { useRef } from "react";
import { MixerStrip, nodeName } from "../api";
import { usePeakLevels } from "../hooks";
import { useStudio } from "../studio";
import { clamp, clamp01, FLOOR_DB, formatDb, formatPan } from "../util";

const MIN_DB = FLOOR_DB;
const MAX_DB = 12;
/** Fader travel is dB-linear above -24 dB and compressed below, like a console. */
const dbToPos = (db: number) => (db <= -24 ? ((db - MIN_DB) / 36) * 0.25 : 0.25 + ((db + 24) / (MAX_DB + 24)) * 0.75);
const posToDb = (p: number) => (p <= 0.25 ? MIN_DB + (p / 0.25) * 36 : -24 + ((p - 0.25) / 0.75) * (MAX_DB + 24));

function StripMeter({ mixer, source }: { mixer: number; source: number }) {
  const { levels: lv } = usePeakLevels((t) => t?.strips.find((s) => s.mixer === mixer && s.source === source)?.peak);
  return (
    <div className="strip-meter">
      {lv.map((v, i) => (
        <div className="strip-meter-lane" key={i}>
          <div className="strip-meter-fill" style={{ height: `${v * 100}%` }} />
        </div>
      ))}
    </div>
  );
}

function VFader({ db, onChange }: { db: number; onChange: (db: number) => void }) {
  const track = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; pos: number } | null>(null);
  const pos = dbToPos(db);
  const set = (p: number) => onChange(Math.round(posToDb(clamp01(p)) * 10) / 10);

  return (
    <div
      ref={track}
      className="vfader"
      onPointerDown={(e) => {
        (e.currentTarget as Element).setPointerCapture(e.pointerId);
        const box = track.current!.getBoundingClientRect();
        const clicked = 1 - (e.clientY - box.top) / box.height;
        // Grab the cap where it is (no jump); clicking the track elsewhere jumps there.
        const onCap = Math.abs(clicked - pos) < 0.08;
        if (!onCap) set(clicked);
        drag.current = { y: e.clientY, pos: onCap ? pos : clicked };
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const box = track.current!.getBoundingClientRect();
        const scale = e.shiftKey ? 4 : 1;
        set(drag.current.pos + (drag.current.y - e.clientY) / box.height / scale);
      }}
      onPointerUp={() => (drag.current = null)}
      onPointerCancel={() => (drag.current = null)}
      onDoubleClick={() => onChange(0)}
      onWheel={(e) => onChange(clamp(Math.round((db - e.deltaY / 40) * 10) / 10, MIN_DB, MAX_DB))}
      title="Drag to set level (Shift = fine). Double-click for 0 dB."
    >
      <span className="vfader-unity" style={{ bottom: `${dbToPos(0) * 100}%` }} />
      <div className="vfader-fill" style={{ height: `${pos * 100}%` }} />
      <div className="vfader-cap" style={{ bottom: `${pos * 100}%` }} />
    </div>
  );
}

export default function MixerStrips({ mixer, strips }: { mixer: number; strips: MixerStrip[] }) {
  const { studio, actions } = useStudio();

  if (strips.length === 0) {
    return <p className="mixer-empty">Connect cables into this mixer — each one gets its own strip.</p>;
  }

  return (
    <div className="mixer nodrag">
      {strips.map((st) => (
        <div key={st.source} className={`strip ${st.mute ? "muted" : ""}`}>
          <input
            className="pan"
            type="range"
            min={-1}
            max={1}
            step={0.01}
            value={st.pan}
            style={{ "--pct": `${((st.pan + 1) / 2) * 100}%` } as React.CSSProperties}
            onChange={(e) => {
              const v = Number(e.target.value);
              actions.setMixerStrip(mixer, st.source, "pan", Math.abs(v) < 0.04 ? 0 : v); // center detent
            }}
            onDoubleClick={() => actions.setMixerStrip(mixer, st.source, "pan", 0)}
            title="Pan (double-click to center)"
          />
          <span className="strip-pan mono">{formatPan(st.pan)}</span>
          <button
            className={`strip-mute ${st.mute ? "on" : ""}`}
            onClick={() => actions.setMixerStrip(mixer, st.source, "mute", st.mute ? 0 : 1)}
            title={st.mute ? "Unmute" : "Mute"}
          >
            M
          </button>
          <div className="strip-body">
            <VFader db={st.gainDb} onChange={(db) => actions.setMixerStrip(mixer, st.source, "gainDb", db)} />
            <StripMeter mixer={mixer} source={st.source} />
          </div>
          <span className="strip-db mono">{formatDb(st.gainDb)}</span>
          <span className="strip-name" title={nodeName(studio.nodes, st.source)}>
            {nodeName(studio.nodes, st.source)}
          </span>
        </div>
      ))}
    </div>
  );
}
