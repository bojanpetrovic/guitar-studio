import { Cpu, Settings2 } from "lucide-react";
import { useStudio } from "../App";
import { useTelemetry } from "../api";
import Meter from "./Meter";
import Logo from "./Logo";

function CpuReadout() {
  const cpu = useTelemetry((t) => t?.cpuLoad ?? 0);
  const pct = Math.round(cpu * 100);
  return (
    <div className={`chip ${pct > 70 ? "warn" : ""}`} title="Audio thread load">
      <Cpu size={14} />
      <span className="mono">{pct}%</span>
    </div>
  );
}

export default function TitleBar() {
  const { studio, actions } = useStudio();
  const a = studio.audio;

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <Logo size={22} />
        <span className="brand-name">Guitar Studio</span>
      </div>

      <div className="titlebar-center" data-tauri-drag-region>
        <button className={`chip audio-chip ${a ? "ok" : "bad"}`} onClick={actions.openSettings}>
          <span className="dot" />
          {a ? (
            <>
              <span className="truncate">{a.inputDevice}</span>
              <span className="sep">→</span>
              <span className="truncate">{a.outputDevice}</span>
              <span className="mono muted">
                {(a.sampleRate / 1000).toFixed(1)} kHz · {a.bufferSize} · {a.latencyMs.toFixed(1)} ms
              </span>
            </>
          ) : (
            <span>Audio off — click to configure</span>
          )}
        </button>
      </div>

      <div className="titlebar-right">
        <CpuReadout />
        <div className="master-meter" title="Main output">
          <span className="micro-label">OUT</span>
          <Meter id="master" />
        </div>
        <button className="icon-btn" onClick={actions.openSettings} aria-label="Audio settings">
          <Settings2 size={18} />
        </button>
      </div>
    </header>
  );
}
