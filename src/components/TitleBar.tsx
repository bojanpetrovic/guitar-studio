import { useEffect, useState } from "react";
import { Cpu, LayoutDashboard, Loader, PanelLeftClose, PanelLeftOpen, Settings2, Workflow } from "lucide-react";
import { IS_PLUGIN, on } from "../transport";
import { useStudio } from "../studio";
import { useTelemetry } from "../api";
import Meter from "./Meter";
import Logo from "./Logo";
import PresetMenu from "./PresetMenu";
import ThemeMenu from "./ThemeMenu";
import PhoneMenu from "./PhoneMenu";
import MuteButton from "./MuteButton";
import { toggleSidebar, useSidebarOpen } from "../sidebar";

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

/** "Preloading X" while a neighbouring preset's plugin starts in the background (the
 *  window may pause for a moment while it does). */
function Warming() {
  const [name, setName] = useState<string | null>(null);
  useEffect(() => on<string | null>("warming", setName), []);
  if (!name) return null;
  return (
    <span className="chip warming" title="Getting the previous / next preset ready so switching is instant">
      <Loader size={12} className="spin" />
      <span className="truncate">Preloading {name}</span>
    </span>
  );
}

/** Shows / hides the plugin list (rig view; the pedalboard has none). ⌘B. */
function SidebarToggle() {
  const { view } = useStudio();
  const open = useSidebarOpen();
  if (view !== "rig") return null;
  return (
    <button className="icon-btn ghost sidebar-toggle" onClick={toggleSidebar} title={`${open ? "Hide" : "Show"} plugins (⌘B)`}>
      {open ? <PanelLeftClose size={17} /> : <PanelLeftOpen size={17} />}
    </button>
  );
}

function ViewSwitch() {
  const { view, actions } = useStudio();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "1") actions.setView("rig");
      else if (e.key === "2") actions.setView("board");
      else if (e.key.toLowerCase() === "b" && view === "rig") toggleSidebar();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [actions, view]);
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
      <SidebarToggle />
      <PresetMenu />
      <ViewSwitch />
      <Warming />

      <div className="titlebar-center" data-tauri-drag-region>
        {IS_PLUGIN ? (
          // Inside a DAW, the DAW runs the audio: nothing to configure here.
          <span className={`chip audio-chip ${a ? "ok" : "bad"}`} title="The DAW runs the audio">
            <span className="dot" />
            {a ? (
              <span className="mono muted">
                DAW · {(a.sampleRate / 1000).toFixed(1)} kHz · {a.inputChannels} ch in
              </span>
            ) : (
              <span>Waiting for the DAW to start audio</span>
            )}
          </span>
        ) : (
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
        )}
      </div>

      <div className="titlebar-right">
        <CpuReadout />
        <div className="master-meter" title="Main output">
          <span className="micro-label">OUT</span>
          <Meter id="master" />
        </div>
        <MuteButton />
        {!IS_PLUGIN && <PhoneMenu />}
        <ThemeMenu />
        {!IS_PLUGIN && (
          <button className="icon-btn" onClick={actions.openSettings} aria-label="Audio settings">
            <Settings2 size={18} />
          </button>
        )}
      </div>
    </header>
  );
}
