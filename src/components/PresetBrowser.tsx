// The presets panel (left sidebar): every saved rig, to load, rename or delete, plus
// "Save as…" and "New rig". Saving the current preset is in the title bar (⌘S).
import { FormEvent, useEffect, useMemo, useState } from "react";
import { Bookmark, Check, FilePlus2, Pencil, Save, Search, Trash2 } from "lucide-react";
import { useStudio } from "../studio";
import { setSidebar } from "../sidebar";
import { timeAgo } from "../util";
import { keys } from "../platform";

type Mode =
  | { kind: "idle" }
  | { kind: "saveAs"; value: string; confirmReplace: boolean }
  | { kind: "confirmNew" }
  | { kind: "rename"; from: string; value: string }
  | { kind: "delete"; name: string };

const IDLE: Mode = { kind: "idle" };

// "Save as…" can be asked for from elsewhere (⇧⌘S, or ⌘S on an untitled rig).
const saveAsRequests = new Set<() => void>();
export function startSaveAs() {
  setSidebar("left", true);
  saveAsRequests.forEach((start) => start());
}

export default function PresetBrowser() {
  const { studio, presets, actions } = useStudio();
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<Mode>(IDLE);
  const current = presets.current;

  useEffect(() => {
    const start = () => setMode({ kind: "saveAs", value: current ?? "", confirmReplace: false });
    saveAsRequests.add(start);
    return () => void saveAsRequests.delete(start);
  }, [current]);

  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return presets.presets;
    return presets.presets.filter(
      (p) => p.name.toLowerCase().includes(q) || p.plugins.some((name) => name.toLowerCase().includes(q)),
    );
  }, [presets.presets, query]);

  const submitSaveAs = async (e: FormEvent) => {
    e.preventDefault();
    if (mode.kind !== "saveAs" || !mode.value.trim()) return;
    const result = await actions.savePreset(mode.value, mode.confirmReplace);
    if (result === "exists") setMode({ ...mode, confirmReplace: true });
    else if (result === "ok") setMode(IDLE);
  };

  const submitRename = async (e: FormEvent) => {
    e.preventDefault();
    if (mode.kind !== "rename" || !mode.value.trim()) return;
    if (await actions.renamePreset(mode.from, mode.value)) setMode(IDLE);
  };

  const startNew = () => {
    if (studio.nodes.length > 2) setMode({ kind: "confirmNew" });
    else actions.newPreset();
  };

  // Esc backs out of a form or confirmation.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && mode.kind !== "idle") {
      e.stopPropagation();
      setMode(IDLE);
    }
  };

  return (
    <div className="browser presets-panel" onKeyDown={onKeyDown}>
      <div className="panel-head">
        <h2>Presets</h2>
        <div className="panel-head-actions">
          <button className="icon-btn" onClick={startNew} title="New empty rig">
            <FilePlus2 size={16} />
          </button>
          <button
            className="icon-btn"
            onClick={() => setMode({ kind: "saveAs", value: current ?? "", confirmReplace: false })}
            title={keys("Save as… (⇧⌘S)")}
          >
            <Save size={16} />
          </button>
        </div>
      </div>

      {mode.kind === "saveAs" && (
        <form className="panel-form" onSubmit={submitSaveAs}>
          <label className="micro-label" htmlFor="preset-name">
            Save rig as
          </label>
          <input
            id="preset-name"
            className="text-input"
            autoFocus
            placeholder="e.g. Djent rhythm, Ambient clean"
            maxLength={64}
            value={mode.value}
            onChange={(e) => setMode({ ...mode, value: e.target.value, confirmReplace: false })}
          />
          {mode.confirmReplace && (
            <p className="panel-warn">“{mode.value.trim()}” already exists. Replace it with the current rig?</p>
          )}
          <div className="panel-form-actions">
            <button type="button" className="btn small" onClick={() => setMode(IDLE)}>
              Cancel
            </button>
            <button type="submit" className={`btn small ${mode.confirmReplace ? "danger" : "primary"}`} disabled={!mode.value.trim()}>
              {mode.confirmReplace ? "Replace" : "Save"}
            </button>
          </div>
        </form>
      )}

      {mode.kind === "confirmNew" && (
        <div className="panel-form">
          <p className="panel-warn">Start a new empty rig? Anything not saved{current ? ` to “${current}”` : ""} will be lost.</p>
          <div className="panel-form-actions">
            <button className="btn small" onClick={() => setMode(IDLE)}>
              Cancel
            </button>
            <button
              className="btn small danger"
              autoFocus
              onClick={() => {
                setMode(IDLE);
                actions.newPreset();
              }}
            >
              New rig
            </button>
          </div>
        </div>
      )}

      {presets.presets.length > 0 && (
        <div className="search">
          <Search size={15} />
          <input placeholder="Search presets or plugins" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
      )}

      <div className="section-label">
        {query ? `${list.length} of ${presets.presets.length}` : `${presets.presets.length} saved`}
        <span className="muted"> · click to load</span>
      </div>

      <div className="plugin-list presets-list">
        {presets.presets.length === 0 && (
          <div className="empty">
            <Bookmark size={26} />
            <p>{keys("No presets yet. Build a rig, then save it with ⌘S.")}</p>
          </div>
        )}
        {list.map((p) => {
          const active = p.name === current;
          if (mode.kind === "rename" && mode.from === p.name) {
            return (
              <form key={p.name} className="preset-row editing" onSubmit={submitRename}>
                <input
                  className="text-input"
                  autoFocus
                  maxLength={64}
                  value={mode.value}
                  onFocus={(e) => e.target.select()}
                  onChange={(e) => setMode({ ...mode, value: e.target.value })}
                  onBlur={() => setMode(IDLE)}
                  aria-label={`Rename “${p.name}”`}
                />
              </form>
            );
          }
          if (mode.kind === "delete" && mode.name === p.name) {
            return (
              <div key={p.name} className="preset-row confirm">
                <span className="preset-text">
                  <span className="preset-row-name">Delete “{p.name}”?</span>
                  <span className="preset-chain">This can’t be undone.</span>
                </span>
                <button className="btn small" onClick={() => setMode(IDLE)}>
                  Cancel
                </button>
                <button
                  className="btn small danger"
                  autoFocus
                  onClick={async () => {
                    await actions.deletePreset(p.name);
                    setMode(IDLE);
                  }}
                >
                  Delete
                </button>
              </div>
            );
          }
          return (
            <div
              key={p.name}
              className={`preset-row ${active ? "active" : ""}`}
              onClick={() => actions.loadPreset(p.name)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === "Enter" && actions.loadPreset(p.name)}
              title={p.plugins.length ? p.plugins.join(" → ") : "No plugins"}
            >
              <span className="preset-check">{active && <Check size={14} />}</span>
              <span className="preset-text">
                <span className="preset-row-name">{p.name}</span>
                <span className="preset-chain">
                  {p.plugins.length ? p.plugins.join(" → ") : "No plugins"} · {timeAgo(p.savedAt)}
                </span>
              </span>
              <span className="row-actions" onClick={(e) => e.stopPropagation()}>
                <button className="icon-btn ghost" title="Rename" onClick={() => setMode({ kind: "rename", from: p.name, value: p.name })}>
                  <Pencil size={13} />
                </button>
                <button className="icon-btn ghost danger" title="Delete" onClick={() => setMode({ kind: "delete", name: p.name })}>
                  <Trash2 size={13} />
                </button>
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
