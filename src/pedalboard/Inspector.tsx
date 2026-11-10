import { useMemo, useState } from "react";
import { Copy, Search, Trash2, WandSparkles, Eraser } from "lucide-react";
import { useStudio } from "../App";
import { nodeName, NodeView } from "../api";
import { useResolved, useRuntime } from "./runtime";
import {
  ACCENTS,
  Binding,
  BINDING_LABEL,
  BindingKind,
  Board,
  BoardSurface,
  ENCLOSURES,
  Widget,
  WIDGET_META,
} from "./types";

interface Props {
  board: Board;
  selected: Widget | null;
  onChange: (w: Widget) => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onBoardChange: (b: Board) => void;
  onGenerate: () => void;
}

const nodeLabel = (n: NodeView, all: NodeView[]) => nodeName(all, n.id);

/** Which rig nodes make sense for a binding kind. */
function candidates(kind: BindingKind, nodes: NodeView[]) {
  switch (kind) {
    case "param":
      return nodes.filter((n) => n.kind === "plugin");
    case "bypass":
      return nodes.filter((n) => n.kind === "plugin" || n.kind === "gain");
    case "mix":
      return nodes.filter((n) => n.kind === "mixer");
    case "level":
      return nodes;
    case "meter":
      return nodes;
    default:
      return [];
  }
}

function defaultBinding(kind: BindingKind, nodes: NodeView[], widget: Widget): Binding {
  const first = candidates(kind, nodes)[0];
  switch (kind) {
    case "mix":
      return first
        ? { kind, node: first.id, source: first.strips[0]?.source ?? -1, field: widget.type === "footswitch" ? "mute" : "gainDb" }
        : { kind: "none" };
    case "param":
      return first ? { kind, node: first.id, paramId: -1, paramName: "" } : { kind: "none" };
    case "bypass":
    case "level":
      return first ? { kind, node: first.id } : { kind: "none" };
    case "meter":
      return { kind, node: "master" };
    default:
      return { kind } as Binding;
  }
}

function ParamPicker({ node, value, onPick }: { node: number; value: number; onPick: (id: number, name: string) => void }) {
  const rt = useRuntime();
  const [q, setQ] = useState("");
  const params = rt.params(node);
  const list = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (params ?? []).filter((p) => !needle || p.name.toLowerCase().includes(needle)).slice(0, 200);
  }, [params, q]);

  if (!params) return <p className="muted small">Loading parameters…</p>;
  return (
    <div className="param-picker">
      <div className="search small">
        <Search size={13} />
        <input placeholder={`Search ${params.length} parameters`} value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      <div className="param-list">
        {list.map((p) => (
          <button key={p.id} className={`param-item ${p.id === value ? "on" : ""}`} onClick={() => onPick(p.id, p.name)}>
            <span className="truncate">{p.name}</span>
            <span className="muted mono">
              {p.steps > 0 ? `${p.steps + 1} steps` : p.unit || (p.min === 0 && p.max === 1 ? "%" : "")}
            </span>
          </button>
        ))}
        {list.length === 0 && <p className="muted small">No matching parameters.</p>}
      </div>
    </div>
  );
}

function NumberField({ label, value, onChange, step = 1 }: { label: string; value: number | undefined; onChange: (v: number | undefined) => void; step?: number }) {
  return (
    <label className="insp-field">
      <span>{label}</span>
      <input
        type="number"
        className="text-input small"
        step={step}
        value={value ?? ""}
        placeholder="auto"
        onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
      />
    </label>
  );
}

function BindingStatus({ w }: { w: Widget }) {
  const r = useResolved(w);
  if (w.binding.kind === "none") return <p className="insp-status warn">Not connected to anything yet.</p>;
  return <p className={`insp-status ${r.ok ? "ok" : "warn"}`}>{r.ok ? `→ ${r.target}` : `Missing: ${r.target}`}</p>;
}

export default function Inspector({ board, selected: w, onChange, onDelete, onDuplicate, onBoardChange, onGenerate }: Props) {
  const { studio } = useStudio();

  if (!w) {
    return (
      <aside className="inspector">
        <h3>Board</h3>
        <div className="insp-row">
          <NumberField label="Width" value={board.width} step={10} onChange={(v) => v && onBoardChange({ ...board, width: Math.max(400, v) })} />
          <NumberField label="Height" value={board.height} step={10} onChange={(v) => v && onBoardChange({ ...board, height: Math.max(200, v) })} />
        </div>
        <label className="insp-field">
          <span>Surface</span>
          <div className="segmented small">
            {(["slate", "carbon", "wood", "steel"] as BoardSurface[]).map((s) => (
              <button key={s} className={board.surface === s ? "on" : ""} onClick={() => onBoardChange({ ...board, surface: s })}>
                {s[0].toUpperCase() + s.slice(1)}
              </button>
            ))}
          </div>
        </label>
        <button className="btn" onClick={onGenerate}>
          <WandSparkles size={14} /> Build from rig
        </button>
        <p className="muted small">
          Creates a pedal for every plugin in your chain, each with an on/off footswitch, its two main knobs and an amp/channel selector where it has one, plus tap tempo, a tuner and an output meter.
        </p>
        {board.widgets.length > 0 && (
          <button className="btn danger" onClick={() => onBoardChange({ ...board, widgets: [] })}>
            <Eraser size={14} /> Clear board
          </button>
        )}
        <p className="muted small hint-keys">
          Select a control to wire it up. Drag to move, pull the corner to resize, ⌫ to delete, ⌘D to duplicate, arrows to nudge.
        </p>
      </aside>
    );
  }

  const meta = WIDGET_META[w.type];
  const set = (patch: Partial<Widget>) => onChange({ ...w, ...patch });
  const setOpt = (patch: Partial<Widget["options"]>) => onChange({ ...w, options: { ...w.options, ...patch } });
  const b = w.binding;
  const nodes = candidates(b.kind, studio.nodes);
  const swatches = w.type === "panel" ? ENCLOSURES : ACCENTS;

  return (
    <aside className="inspector">
      <div className="insp-head">
        <h3>{meta.name}</h3>
        <div className="row gap">
          <button className="icon-btn ghost" title="Duplicate (⌘D)" onClick={onDuplicate}>
            <Copy size={14} />
          </button>
          <button className="icon-btn ghost danger" title="Delete (⌫)" onClick={onDelete}>
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      <label className="insp-field">
        <span>{w.type === "panel" ? "Pedal name" : "Label"}</span>
        <input className="text-input small" value={w.label} maxLength={40} onChange={(e) => set({ label: e.target.value })} />
      </label>

      <div className="insp-field">
        <span>{w.type === "panel" ? "Enclosure" : "Color"}</span>
        <div className="swatches">
          {swatches.map((c) => (
            <button key={c} className={`swatch ${w.color === c ? "on" : ""}`} style={{ background: c }} onClick={() => set({ color: c })} aria-label={c} />
          ))}
        </div>
      </div>

      {w.type === "label" && (
        <NumberField label="Text size" value={w.options.fontSize} onChange={(v) => setOpt({ fontSize: v })} />
      )}

      {meta.bindings.length > 0 && (
        <div className="insp-section">
          <h4>Controls</h4>
          <label className="insp-field">
            <span>Target</span>
            <select
              value={b.kind}
              onChange={(e) => set({ binding: defaultBinding(e.target.value as BindingKind, studio.nodes, w) })}
            >
              {meta.bindings.map((k) => (
                <option key={k} value={k}>
                  {BINDING_LABEL[k]}
                </option>
              ))}
            </select>
          </label>

          {(b.kind === "param" || b.kind === "bypass" || b.kind === "level") && (
            <label className="insp-field">
              <span>{b.kind === "param" ? "Plugin" : "Node"}</span>
              <select
                value={b.node}
                onChange={(e) => {
                  const node = Number(e.target.value);
                  set({ binding: b.kind === "param" ? { kind: "param", node, paramId: -1, paramName: "" } : { ...b, node } });
                }}
              >
                {!nodes.some((n) => n.id === b.node) && <option value={b.node}>(missing)</option>}
                {nodes.map((n) => (
                  <option key={n.id} value={n.id}>
                    {nodeLabel(n, studio.nodes)}
                  </option>
                ))}
              </select>
            </label>
          )}

          {b.kind === "mix" && (
            <>
              <label className="insp-field">
                <span>Mixer</span>
                <select
                  value={b.node}
                  onChange={(e) => {
                    const m = nodes.find((n) => n.id === Number(e.target.value));
                    set({ binding: { ...b, node: Number(e.target.value), source: m?.strips[0]?.source ?? -1 } });
                  }}
                >
                  {nodes.length === 0 && <option value={b.node}>No mixers in the rig</option>}
                  {nodes.map((n, i) => (
                    <option key={n.id} value={n.id}>
                      Mixer {nodes.length > 1 ? i + 1 : ""} ({n.strips.length} inputs)
                    </option>
                  ))}
                </select>
              </label>
              <label className="insp-field">
                <span>Input</span>
                <select value={b.source} onChange={(e) => set({ binding: { ...b, source: Number(e.target.value) } })}>
                  {!nodes.find((n) => n.id === b.node)?.strips.some((st) => st.source === b.source) && (
                    <option value={b.source}>(not connected)</option>
                  )}
                  {nodes
                    .find((n) => n.id === b.node)
                    ?.strips.map((st) => (
                      <option key={st.source} value={st.source}>
                        {nodeName(studio.nodes, st.source)}
                      </option>
                    ))}
                </select>
              </label>
              {w.type !== "footswitch" && (
                <label className="insp-field">
                  <span>Control</span>
                  <div className="segmented small">
                    {(["gainDb", "pan"] as const).map((f) => (
                      <button key={f} className={b.field === f ? "on" : ""} onClick={() => set({ binding: { ...b, field: f } })}>
                        {f === "gainDb" ? "Volume" : "Pan"}
                      </button>
                    ))}
                  </div>
                </label>
              )}
            </>
          )}

          {b.kind === "meter" && (
            <label className="insp-field">
              <span>Measure</span>
              <select value={String(b.node)} onChange={(e) => set({ binding: { kind: "meter", node: e.target.value === "master" ? "master" : Number(e.target.value) } })}>
                <option value="master">Main output</option>
                {nodes.map((n) => (
                  <option key={n.id} value={n.id}>
                    {nodeLabel(n, studio.nodes)} output
                  </option>
                ))}
              </select>
            </label>
          )}

          {b.kind === "param" && nodes.some((n) => n.id === b.node) && (
            <div className="insp-field">
              <span>Parameter{b.paramName ? `: ${b.paramName}` : ""}</span>
              <ParamPicker
                node={b.node}
                value={b.paramId}
                onPick={(paramId, paramName) => set({ binding: { ...b, paramId, paramName }, label: w.label === WIDGET_META[w.type].name || ["Stomp", "Knob", "Fader", "Mode"].includes(w.label) ? paramName.slice(0, 18) : w.label })}
              />
            </div>
          )}

          <BindingStatus w={w} />

          {w.type === "footswitch" && b.kind !== "tap" && b.kind !== "none" && (
            <label className="check-row">
              <input type="checkbox" checked={!!w.options.momentary} onChange={(e) => setOpt({ momentary: e.target.checked })} />
              <span>Momentary (only while held)</span>
            </label>
          )}
          {w.type === "footswitch" && b.kind === "param" && (
            <div className="insp-row">
              <NumberField label="On value" step={0.01} value={w.options.onValue} onChange={(v) => setOpt({ onValue: v })} />
              <NumberField label="Off value" step={0.01} value={w.options.offValue} onChange={(v) => setOpt({ offValue: v })} />
            </div>
          )}
          {(w.type === "knob" || w.type === "fader") && b.kind !== "none" && (
            <div className="insp-row">
              <NumberField label="Range from" step={0.01} value={w.options.min} onChange={(v) => setOpt({ min: v })} />
              <NumberField label="to" step={0.01} value={w.options.max} onChange={(v) => setOpt({ max: v })} />
            </div>
          )}
          {w.type === "selector" && (
            <label className="insp-field">
              <span>Positions (comma separated)</span>
              <input
                className="text-input small"
                value={(w.options.labels ?? []).join(", ")}
                placeholder="Clean, Crunch, Lead"
                onChange={(e) => setOpt({ labels: e.target.value.split(",").map((x) => x.trim()) })}
              />
            </label>
          )}
        </div>
      )}

      <div className="insp-row">
        <NumberField label="W" value={w.w} step={10} onChange={(v) => v && set({ w: Math.max(meta.minW, v) })} />
        <NumberField label="H" value={w.h} step={10} onChange={(v) => v && set({ h: Math.max(meta.minH, v) })} />
      </div>
    </aside>
  );
}
