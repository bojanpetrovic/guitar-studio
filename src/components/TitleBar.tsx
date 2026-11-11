import { useEffect } from "react";
import { Cpu, LayoutDashboard, Settings2, Workflow } from "lucide-react";
import { useStudio } from "../App";
import { useTelemetry } from "../api";
import Meter from "./Meter";
import Logo from "./Logo";
import PresetMenu from "./PresetMenu";
import ThemeMenu from "./ThemeMenu";
import PhoneMenu from "./PhoneMenu";

function CpuReadout() {
  const cpu = useTelemetry((t) => t?.cpuLoad ?? 0);
  const pct = Math.round(cpu * 100);
  return (
    <div className={`chip cpu-chip ${pct > 70 ? "warn" : ""}`} title="Audio thread load">
      <Cpu size={14} />
      <span className="mono cpu-value">{pct}%</span>
    </div>
  );
}

function ViewSwitch() {
  const { view, actions } = useStudio();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "1") actions.setView("rig");
      else if (e.key === "2") actions.setView("board");
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actions]);
  return (
    <div className="segmented small view-switch">
      <button className={view === "rig" ? "on" : ""} onClick={() => actions.setView("rig")} title="Rig (⌘1)">
        <Workflow size={13} /> Rig
      </button>
      <button className={view === "board" ? "on" : ""} onClick={() => actions.setView("board")} title="Pedalboard (⌘2)">
        <LayoutDashboard size={13} /> Pedalboard
      </button>
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
      <PresetMenu />
      <ViewSwitch />

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
        <PhoneMenu />
        <ThemeMenu />
        <button className="icon-btn" onClick={actions.openSettings} aria-label="Audio settings">
          <Settings2 size={18} />
        </button>
      </div>
    </header>
  );
}
