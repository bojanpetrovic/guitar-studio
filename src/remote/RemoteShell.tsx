// The phone: just the pedalboard, a preset switcher and the tuner / metronome dock.
// Everything goes to the desktop app over the local network (see transport.ts).
import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, Maximize2, Minimize2, RotateCw, WifiOff } from "lucide-react";
import Dock from "../components/Dock";
import Logo from "../components/Logo";
import PedalboardView from "../pedalboard/PedalboardView";
import { Link, useLink } from "../transport";
import { usePresetStep, useStudio } from "../studio";
import { parseBoard } from "../pedalboard/types";

const MESSAGES: Record<Exclude<Link, "open">, { title: string; body: string }> = {
  connecting: { title: "Connecting to Guitar Studio…", body: "" },
  lost: {
    title: "Reconnecting…",
    body: "Is Guitar Studio still running on your computer, and is this phone on the same Wi-Fi?",
  },
  denied: {
    title: "This link no longer works",
    body: "Scan the QR code in Guitar Studio again (the phone icon in the title bar).",
  },
  nolink: {
    title: "Scan the QR code to connect",
    body: "In Guitar Studio on your computer, click the phone icon in the title bar and scan the code.",
  },
};

function LinkMessage({ link }: { link: Exclude<Link, "open"> }) {
  const m = MESSAGES[link];
  return (
    <div className="remote-message">
      {link === "lost" || link === "denied" ? <WifiOff size={26} /> : <Logo size={34} />}
      <h3>{m.title}</h3>
      {m.body && <p>{m.body}</p>}
      {link === "connecting" && <span className="remote-spinner" />}
    </div>
  );
}

/** Shown until the first rig state arrives. */
export function RemoteBoot() {
  const link = useLink();
  return (
    <div className="boot remote-boot">
      <LinkMessage link={link === "open" ? "connecting" : link} />
    </div>
  );
}

function useFullscreen() {
  const [on, setOn] = useState(!!document.fullscreenElement);
  useEffect(() => {
    const sync = () => setOn(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", sync);
    return () => document.removeEventListener("fullscreenchange", sync);
  }, []);
  // iPhone Safari has no fullscreen for pages ("Add to Home Screen" does that instead).
  const supported = !!document.fullscreenEnabled;
  const toggle = () =>
    (on ? document.exitFullscreen() : document.documentElement.requestFullscreen({ navigationUI: "hide" })).catch(() => {});
  return { on, supported, toggle };
}

export default function RemoteShell() {
  const link = useLink();
  const { step, count, current } = usePresetStep();
  const fs = useFullscreen();
  const { studio } = useStudio();
  const board = parseBoard(studio.pedalboard);
  const wide = board.widgets.length > 0 && board.width / board.height > 1.3;

  useEffect(() => {
    document.documentElement.classList.add("remote-mode");
  }, []);

  return (
    <div className={`remote link-${link}`}>
      <header className="remote-bar">
        <span className={`remote-dot ${link}`} title={link === "open" ? "Connected" : "Not connected"} />
        <button className="icon-btn" onClick={() => step(-1)} disabled={count === 0} aria-label="Previous preset">
          <ChevronLeft size={20} />
        </button>
        <span className="remote-preset">{current ?? "Untitled rig"}</span>
        <button className="icon-btn" onClick={() => step(1)} disabled={count === 0} aria-label="Next preset">
          <ChevronRight size={20} />
        </button>
        {fs.supported && (
          <button className="icon-btn ghost remote-fs" onClick={fs.toggle} aria-label={fs.on ? "Exit full screen" : "Full screen"}>
            {fs.on ? <Minimize2 size={17} /> : <Maximize2 size={17} />}
          </button>
        )}
      </header>
      <main className="remote-board">
        <span className={`remote-rotate ${wide ? "wide" : ""}`}>
          <RotateCw size={13} /> Turn your phone sideways for a bigger board
        </span>
        <PedalboardView />
      </main>
      <Dock />
      {link !== "open" && (
        <div className="remote-overlay">
          <LinkMessage link={link} />
        </div>
      )}
    </div>
  );
}
