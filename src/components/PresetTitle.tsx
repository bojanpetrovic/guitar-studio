// Title bar: the current preset's name (click: show / hide the presets panel) and Save.
// ⌘S saves the current preset (or asks for a name), ⇧⌘S saves as.
import { useCallback, useEffect } from "react";
import { Bookmark, Save } from "lucide-react";
import { useStudio } from "../studio";
import { toggleSidebar } from "../sidebar";
import { startSaveAs } from "./PresetBrowser";

export default function PresetTitle() {
  const { presets, actions } = useStudio();
  const current = presets.current;

  const save = useCallback(() => {
    if (current) actions.savePreset(current);
    else startSaveAs();
  }, [current, actions]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      if (e.shiftKey) startSaveAs();
      else save();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [save]);

  return (
    <div className="preset">
      <button className="preset-trigger" onClick={() => toggleSidebar("left")} title="Presets (⌘P)">
        <Bookmark size={14} />
        <span className={`preset-name ${current ? "" : "muted"}`}>{current ?? "Untitled rig"}</span>
      </button>
      <button className="icon-btn" onClick={save} title={current ? `Save “${current}” (⌘S)` : "Save as… (⌘S)"}>
        <Save size={16} />
      </button>
    </div>
  );
}
