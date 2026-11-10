// Scene buttons: click (or press 1-9) to switch, double-click to rename or delete,
// + to add a scene from the current sound. Shown on the rig canvas and under the
// pedalboard. Edits always go into the active scene (the engine saves them on switch).
import { FormEvent, useEffect, useRef, useState } from "react";
import { Check, Layers, Plus, Trash2 } from "lucide-react";
import { useStudio } from "../App";
import { MAX_SCENES } from "../api";

/** Height of the board variant, incl. padding (stage mode sizes the window with it). */
export const SCENE_BAR_H = 50;

function RenameField({ index, name, onDone }: { index: number; name: string; onDone: () => void }) {
  const { actions } = useStudio();
  const [value, setValue] = useState(name);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  const commit = async (e?: FormEvent) => {
    e?.preventDefault();
    const v = value.trim();
    if (v && v !== name) await actions.renameScene(index, v);
    onDone();
  };

  return (
    <form className="scene editing" onSubmit={commit}>
      <span className="scene-key mono">{index + 1}</span>
      <input
        ref={input}
        className="scene-input"
        value={value}
        maxLength={40}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => commit()}
        onKeyDown={(e) => e.key === "Escape" && onDone()}
      />
      <button type="submit" className="icon-btn ghost mini" title="Rename" onMouseDown={(e) => e.preventDefault()}>
        <Check size={12} />
      </button>
      <button
        type="button"
        className="icon-btn ghost mini danger"
        title="Delete scene"
        // Keep focus in the field so blur doesn't commit first.
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => {
          actions.deleteScene(index);
          onDone();
        }}
      >
        <Trash2 size={12} />
      </button>
    </form>
  );
}

export default function SceneBar({ variant }: { variant: "canvas" | "board" }) {
  const { studio, actions } = useStudio();
  const [renaming, setRenaming] = useState<number | null>(null);
  const scenes = studio.scenes;
  const active = studio.activeScene;

  return (
    <div className={`scenebar on-${variant} nodrag`} onPointerDown={(e) => e.stopPropagation()}>
      {variant === "canvas" && (
        <span className="scenebar-label micro-label">
          <Layers size={12} /> Scenes
        </span>
      )}
      {scenes.map((name, i) =>
        renaming === i ? (
          <RenameField key={i} index={i} name={name} onDone={() => setRenaming(null)} />
        ) : (
          <button
            key={i}
            className={`scene ${active === i ? "on" : ""}`}
            onClick={() => actions.selectScene(i)}
            onDoubleClick={() => setRenaming(i)}
            title={`${name} — press ${i + 1}. Double-click to rename or delete.`}
          >
            <span className="scene-led" />
            <span className="scene-key mono">{i + 1}</span>
            <span className="scene-name">{name}</span>
          </button>
        ),
      )}
      {scenes.length < MAX_SCENES && (
        <button
          className={`scene add ${scenes.length === 0 ? "first" : ""}`}
          onClick={actions.addScene}
          title="New scene from the current sound"
        >
          <Plus size={13} />
          {scenes.length === 0 && <span>Add scene</span>}
        </button>
      )}
      {scenes.length === 0 && variant === "board" && (
        <span className="scenebar-hint muted">Scenes remember every plugin's settings — switch with 1-9</span>
      )}
    </div>
  );
}
