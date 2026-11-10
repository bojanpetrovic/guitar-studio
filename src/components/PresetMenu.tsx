import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { Bookmark, Check, ChevronDown, FilePlus2, Pencil, Save, Trash2 } from "lucide-react";
import { useStudio } from "../App";

type Mode =
  | { kind: "list" }
  | { kind: "saveAs"; value: string; confirmReplace: boolean }
  | { kind: "rename"; from: string; value: string }
  | { kind: "delete"; name: string }
  | { kind: "confirmNew" };

function ago(secs: number) {
  const d = Date.now() / 1000 - secs;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return new Date(secs * 1000).toLocaleDateString();
}

export default function PresetMenu() {
  const { studio, presets, actions } = useStudio();
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const root = useRef<HTMLDivElement>(null);
  const current = presets.current;

  const close = useCallback(() => {
    setOpen(false);
    setMode({ kind: "list" });
  }, []);

  const openSaveAs = useCallback(() => {
    setOpen(true);
    setMode({ kind: "saveAs", value: current ?? "", confirmReplace: false });
  }, [current]);

  const save = useCallback(() => {
    if (current) actions.savePreset(current);
    else openSaveAs();
  }, [current, actions, openSaveAs]);

  // ⌘S saves, ⇧⌘S saves as.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      if (e.shiftKey) openSaveAs();
      else save();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [save, openSaveAs]);

  // Click outside / Escape closes.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, close]);

  const submitSaveAs = async (e: FormEvent) => {
    e.preventDefault();
    if (mode.kind !== "saveAs" || !mode.value.trim()) return;
    const result = await actions.savePreset(mode.value, mode.confirmReplace);
    if (result === "exists") setMode({ ...mode, confirmReplace: true });
    else if (result === "ok") close();
  };

  const submitRename = async (e: FormEvent) => {
    e.preventDefault();
    if (mode.kind !== "rename" || !mode.value.trim()) return;
    if (await actions.renamePreset(mode.from, mode.value)) setMode({ kind: "list" });
  };

  const load = (name: string) => {
    close();
    actions.loadPreset(name);
  };

  const startNew = () => {
    if (studio.nodes.length > 2) setMode({ kind: "confirmNew" });
    else {
      close();
      actions.newPreset();
    }
  };

  return (
    <div className="preset" ref={root}>
      <button className={`preset-trigger ${open ? "open" : ""}`} onClick={() => (open ? close() : setOpen(true))}>
        <Bookmark size={14} />
        <span className={`preset-name ${current ? "" : "muted"}`}>{current ?? "Untitled rig"}</span>
        <ChevronDown size={14} className="chev" />
      </button>
      <button className="icon-btn" onClick={save} title={current ? `Save “${current}” (⌘S)` : "Save as… (⌘S)"}>
        <Save size={16} />
      </button>

      {open && (
        <div className="popover preset-pop">
          {mode.kind === "list" && (
            <>
              <div className="pop-head">
                <span className="micro-label">Presets</span>
                <span className="muted">{presets.presets.length}</span>
              </div>
              <div className="preset-list">
                {presets.presets.length === 0 && (
                  <div className="pop-empty">No presets yet. Build a rig, then save it with ⌘S.</div>
                )}
                {presets.presets.map((p) => {
                  const active = p.name === current;
                  return (
                    <div
                      key={p.name}
                      className={`preset-row ${active ? "active" : ""}`}
                      onClick={() => load(p.name)}
                      role="button"
                      tabIndex={0}
                      onKeyDown={(e) => e.key === "Enter" && load(p.name)}
                    >
                      <span className="preset-check">{active && <Check size={14} />}</span>
                      <span className="preset-text">
                        <span className="preset-row-name">{p.name}</span>
                        <span className="preset-chain">
                          {p.plugins.length ? p.plugins.join(" → ") : "No plugins"} · {ago(p.savedAt)}
                        </span>
                      </span>
                      <span className="row-actions" onClick={(e) => e.stopPropagation()}>
                        <button
                          className="icon-btn ghost"
                          title="Rename"
                          onClick={() => setMode({ kind: "rename", from: p.name, value: p.name })}
                        >
                          <Pencil size={13} />
                        </button>
                        <button
                          className="icon-btn ghost danger"
                          title="Delete"
                          onClick={() => setMode({ kind: "delete", name: p.name })}
                        >
                          <Trash2 size={13} />
                        </button>
                      </span>
                    </div>
                  );
                })}
              </div>
              <div className="pop-foot">
                <button className="btn small" onClick={startNew}>
                  <FilePlus2 size={13} /> New rig
                </button>
                <button className="btn small primary" onClick={openSaveAs}>
                  <Save size={13} /> Save as…
                </button>
              </div>
            </>
          )}

          {mode.kind === "saveAs" && (
            <form className="pop-form" onSubmit={submitSaveAs}>
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
                <p className="pop-warn">“{mode.value.trim()}” already exists. Replace it with the current rig?</p>
              )}
              <div className="pop-actions">
                <button type="button" className="btn small" onClick={() => setMode({ kind: "list" })}>
                  Cancel
                </button>
                <button type="submit" className={`btn small ${mode.confirmReplace ? "danger" : "primary"}`} disabled={!mode.value.trim()}>
                  {mode.confirmReplace ? "Replace" : "Save"}
                </button>
              </div>
            </form>
          )}

          {mode.kind === "rename" && (
            <form className="pop-form" onSubmit={submitRename}>
              <label className="micro-label" htmlFor="preset-rename">
                Rename “{mode.from}”
              </label>
              <input
                id="preset-rename"
                className="text-input"
                autoFocus
                maxLength={64}
                value={mode.value}
                onFocus={(e) => e.target.select()}
                onChange={(e) => setMode({ ...mode, value: e.target.value })}
              />
              <div className="pop-actions">
                <button type="button" className="btn small" onClick={() => setMode({ kind: "list" })}>
                  Cancel
                </button>
                <button type="submit" className="btn small primary" disabled={!mode.value.trim()}>
                  Rename
                </button>
              </div>
            </form>
          )}

          {mode.kind === "delete" && (
            <div className="pop-form">
              <p className="pop-warn">Delete “{mode.name}”? This can’t be undone.</p>
              <div className="pop-actions">
                <button className="btn small" onClick={() => setMode({ kind: "list" })}>
                  Cancel
                </button>
                <button
                  className="btn small danger"
                  autoFocus
                  onClick={async () => {
                    await actions.deletePreset(mode.name);
                    setMode({ kind: "list" });
                  }}
                >
                  Delete
                </button>
              </div>
            </div>
          )}

          {mode.kind === "confirmNew" && (
            <div className="pop-form">
              <p className="pop-warn">
                Start a new empty rig? Anything not saved{current ? ` to “${current}”` : ""} will be lost.
              </p>
              <div className="pop-actions">
                <button className="btn small" onClick={() => setMode({ kind: "list" })}>
                  Cancel
                </button>
                <button
                  className="btn small danger"
                  autoFocus
                  onClick={() => {
                    close();
                    actions.newPreset();
                  }}
                >
                  New rig
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
