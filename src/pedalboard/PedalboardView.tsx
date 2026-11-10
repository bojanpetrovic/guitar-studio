import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  CircleDot,
  Gauge,
  ListChecks,
  Pencil,
  Play,
  SlidersVertical,
  Square,
  Type,
  AudioWaveform,
  WandSparkles,
  ChevronLeft,
  ChevronRight,
  Workflow,
} from "lucide-react";
import { useStudio } from "../App";
import { api, nodeName, NodeView, ParamInfo } from "../api";
import Inspector from "./Inspector";
import { RuntimeContext, useParamRuntime } from "./runtime";
import { WidgetView } from "./widgets";
import { enterStage, exitStage } from "../stageWindow";
import {
  ACCENTS,
  Board,
  drawOrder,
  ENCLOSURES,
  GRID,
  makeWidget,
  newId,
  parseBoard,
  snap,
  Widget,
  WIDGET_META,
  WidgetType,
} from "./types";

const PALETTE: { type: WidgetType; icon: React.ReactNode }[] = [
  { type: "footswitch", icon: <CircleDot size={15} /> },
  { type: "knob", icon: <Gauge size={15} /> },
  { type: "fader", icon: <SlidersVertical size={15} /> },
  { type: "selector", icon: <ListChecks size={15} /> },
  { type: "meter", icon: <Activity size={15} /> },
  { type: "tuner", icon: <AudioWaveform size={15} /> },
  { type: "label", icon: <Type size={15} /> },
  { type: "panel", icon: <Square size={15} /> },
];

type Drag = { id: string; kind: "move" | "resize"; x: number; y: number; orig: Widget; moved: boolean };

/** The two parameters most likely to matter on a pedal: gain/drive, then level/mix/tone… */
function pickKnobs(params: ParamInfo[]) {
  const priority = ["gain", "drive", "master", "volume", "level", "mix", "wet", "feedback", "time", "tone"];
  const rank = (name: string) => {
    const i = priority.findIndex((k) => name.toLowerCase().includes(k));
    return i < 0 ? priority.length : i;
  };
  return params
    .filter((q) => q.steps === 0 && !q.isBypass && !/bypass|active|phase|sync|link|invert|\beq\b|pan\b/i.test(q.name))
    .map((q, order) => ({ q, order, r: rank(q.name) }))
    .sort((a, b) => a.r - b.r || a.order - b.order)
    .slice(0, 2)
    .map((x) => x.q);
}

/** Plugins in signal order (walking cables from the input), then any stragglers. */
function chainOrder(nodes: NodeView[], edges: { from: number; to: number }[]) {
  const input = nodes.find((n) => n.kind === "input");
  const seen = new Set<number>();
  const order: NodeView[] = [];
  const queue = input ? [input.id] : [];
  while (queue.length) {
    const id = queue.shift()!;
    for (const e of edges.filter((e) => e.from === id)) {
      if (seen.has(e.to)) continue;
      seen.add(e.to);
      const n = nodes.find((x) => x.id === e.to);
      if (n?.kind === "plugin") order.push(n);
      queue.push(e.to);
    }
  }
  return [...order, ...nodes.filter((n) => n.kind === "plugin" && !seen.has(n.id))];
}

/**
 * The only chrome on stage: a slim bar that fades in when the mouse moves and
 * fades out again while you play. ← / → (or Page Up / Page Down, [ / ]) step through presets.
 */
function StageHud({ onEdit, onRig }: { onEdit: () => void; onRig: () => void }) {
  const { presets, actions } = useStudio();
  const [awake, setAwake] = useState(true);
  const hovering = useRef(false);
  const timer = useRef<number | undefined>(undefined);

  const names = presets.presets.map((p) => p.name);
  const at = presets.current ? names.indexOf(presets.current) : -1;
  const step = useCallback(
    (dir: 1 | -1) => {
      if (names.length < 2 && at >= 0) return;
      const next = names[(at + dir + names.length) % names.length];
      if (next && next !== presets.current) actions.loadPreset(next);
    },
    [names, at, presets, actions],
  );

  useEffect(() => {
    const wake = () => {
      setAwake(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => !hovering.current && setAwake(false), 2200);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if ((e.target as HTMLElement)?.closest?.("input, textarea, select")) return;
      if (e.key === "ArrowLeft" || e.key === "PageUp" || e.key === "[") step(-1);
      else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === "]") step(1);
      else return;
      e.preventDefault();
    };
    wake();
    window.addEventListener("pointermove", wake);
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(timer.current);
      window.removeEventListener("pointermove", wake);
      window.removeEventListener("keydown", onKey);
    };
  }, [step]);

  return (
    <div
      className={`stage-hud ${awake ? "awake" : ""}`}
      onPointerEnter={() => (hovering.current = true)}
      onPointerLeave={() => (hovering.current = false)}
    >
      <button className="icon-btn ghost" onClick={() => step(-1)} disabled={names.length === 0} title="Previous preset (←)">
        <ChevronLeft size={16} />
      </button>
      <span className="stage-hud-name" data-tauri-drag-region>
        {presets.current ?? "Untitled rig"}
      </span>
      <button className="icon-btn ghost" onClick={() => step(1)} disabled={names.length === 0} title="Next preset (→)">
        <ChevronRight size={16} />
      </button>
      <span className="stage-hud-sep" />
      <button className="btn small" onClick={onEdit} title="Edit the board (⌘E or Esc)">
        <Pencil size={13} /> Edit
      </button>
      <button className="btn small" onClick={onRig} title="Back to the rig (⌘1)">
        <Workflow size={13} /> Rig
      </button>
    </div>
  );
}

/** Height the floating tuner/metronome dock takes under the board on stage (see .dock in styles.css). */
const STAGE_DOCK_H = 64;

export default function PedalboardView() {
  const { studio, presets, layoutKey, actions } = useStudio();
  const stored = useMemo(() => parseBoard(studio.pedalboard), [studio.pedalboard]);
  const [draft, setDraft] = useState<Board | null>(null);
  const board = draft ?? stored;
  const [mode, setMode] = useState<"play" | "edit">(stored.widgets.length ? "play" : "edit");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const runtime = useParamRuntime(board, studio, true);
  const editing = mode === "edit";
  const selected = board.widgets.find((w) => w.id === selectedId) ?? null;

  // A freshly loaded rig opens ready to play if it has a board, ready to design if not.
  useEffect(() => {
    if (!layoutKey) return;
    setMode(parseBoard(studio.pedalboard).widgets.length ? "play" : "edit");
    setSelectedId(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only on rig swaps
  }, [layoutKey]);

  const commit = useCallback(
    (b: Board) => {
      setDraft(null);
      actions.updateBoard(b);
    },
    [actions],
  );
  const updateWidget = useCallback(
    (w: Widget) => commit({ ...board, widgets: board.widgets.map((x) => (x.id === w.id ? w : x)) }),
    [board, commit],
  );

  // ---- fit the board into the stage
  const stageRef = useRef<HTMLDivElement>(null);
  const [avail, setAvail] = useState({ w: 800, h: 500 });
  useLayoutEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setAvail({ w: e.contentRect.width, h: e.contentRect.height }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // ---- stage mode: playing a board shows only the board, in a frameless window
  const stage = !editing && board.widgets.length > 0;
  useEffect(() => {
    if (!stage) return;
    actions.setStage(true);
    return () => {
      actions.setStage(false);
      exitStage();
    };
  }, [stage, actions]);
  useEffect(() => {
    if (stage) enterStage(board.width / board.height, STAGE_DOCK_H);
  }, [stage, board.width, board.height]);

  // ⌘E flips between playing and designing; Esc leaves the stage for the editor.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = (e.target as HTMLElement)?.closest?.("input, textarea, select");
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "e") {
        e.preventDefault();
        setMode((m) => (m === "edit" ? "play" : "edit"));
        setSelectedId(null);
      } else if (stage && !typing && e.key === "Escape") {
        setMode("edit");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stage]);

  const margin = stage ? 0 : 48;
  const scale = Math.max(0.2, Math.min((avail.w - margin) / board.width, (avail.h - margin) / board.height, stage ? 4 : 1.4));

  // ---- editing gestures
  const drag = useRef<Drag | null>(null);
  const onWidgetDown = (e: React.PointerEvent, w: Widget, kind: Drag["kind"]) => {
    if (!editing) return;
    e.stopPropagation();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    setSelectedId(w.id);
    drag.current = { id: w.id, kind, x: e.clientX, y: e.clientY, orig: w, moved: false };
  };
  const onWidgetMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = (e.clientX - d.x) / scale;
    const dy = (e.clientY - d.y) / scale;
    if (!d.moved && Math.hypot(dx, dy) < 2) return;
    d.moved = true;
    const m = WIDGET_META[d.orig.type];
    const next: Widget =
      d.kind === "move"
        ? {
            ...d.orig,
            x: Math.max(0, Math.min(board.width - d.orig.w, snap(d.orig.x + dx))),
            y: Math.max(0, Math.min(board.height - d.orig.h, snap(d.orig.y + dy))),
          }
        : {
            ...d.orig,
            w: Math.max(m.minW, Math.min(board.width - d.orig.x, snap(d.orig.w + dx))),
            h: Math.max(m.minH, Math.min(board.height - d.orig.y, snap(d.orig.h + dy))),
          };
    setDraft({ ...board, widgets: board.widgets.map((x) => (x.id === d.id ? next : x)) });
  };
  const onWidgetUp = () => {
    if (drag.current?.moved && draft) commit(draft);
    drag.current = null;
  };

  const add = (type: WidgetType) => {
    const m = WIDGET_META[type];
    const n = board.widgets.length;
    const w = makeWidget(type, board.width / 2 - m.w / 2 + (n % 5) * GRID * 2, board.height / 2 - m.h / 2 + (n % 5) * GRID * 2);
    w.color = type === "panel" ? ENCLOSURES[n % ENCLOSURES.length] : ACCENTS[0];
    commit({ ...board, widgets: [...board.widgets, w] });
    setSelectedId(w.id);
  };

  const remove = useCallback(() => {
    if (!selectedId) return;
    commit({ ...board, widgets: board.widgets.filter((w) => w.id !== selectedId) });
    setSelectedId(null);
  }, [board, selectedId, commit]);

  const duplicate = useCallback(() => {
    if (!selected) return;
    const copy = { ...selected, id: newId(), x: snap(selected.x + 20), y: snap(selected.y + 20) };
    commit({ ...board, widgets: [...board.widgets, copy] });
    setSelectedId(copy.id);
  }, [board, selected, commit]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)) return;
      if (!selected) return;
      if (e.key === "Backspace" || e.key === "Delete") {
        e.preventDefault();
        remove();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d") {
        e.preventDefault();
        duplicate();
      } else if (e.key.startsWith("Arrow")) {
        e.preventDefault();
        const step = e.shiftKey ? GRID * 5 : GRID;
        const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
        const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
        updateWidget({
          ...selected,
          x: Math.max(0, Math.min(board.width - selected.w, selected.x + dx)),
          y: Math.max(0, Math.min(board.height - selected.h, selected.y + dy)),
        });
      } else if (e.key === "Escape") {
        setSelectedId(null);
      }
    };
    // On document, so a selected widget claims the arrow keys before the window-level
    // tempo keys see them (those skip events that were already handled).
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [editing, selected, board, remove, duplicate, updateWidget]);

  // ---- "Build from rig"
  const generate = async () => {
    const plugins = chainOrder(studio.nodes, studio.edges);
    const widgets: Widget[] = [];
    let x = 30;
    for (const [i, p] of plugins.entries()) {
      const params = await api.pluginParams(p.id).catch(() => []);
      const knobs = pickKnobs(params);
      const modeParam = params.find(
        (q) => /type|channel|mode|amp\b/i.test(q.name) && q.min === 0 && q.max >= 2 && q.max <= 7 && Number.isInteger(q.max),
      );
      const width = modeParam ? 400 : 260;
      const panel = makeWidget("panel", x, 30);
      Object.assign(panel, { w: width, h: 350, color: ENCLOSURES[i % ENCLOSURES.length], label: (p.plugin?.name ?? "Pedal").toUpperCase().slice(0, 22) });
      widgets.push(panel);
      knobs.forEach((q, k) => {
        const knob = makeWidget("knob", x + 30 + k * 110, 60);
        Object.assign(knob, { label: q.name.slice(0, 16), color: "#f2f2f2", binding: { kind: "param", node: p.id, paramId: q.id, paramName: q.name } });
        widgets.push(knob);
      });
      if (modeParam) {
        const sel = makeWidget("selector", x + 250, 50);
        Object.assign(sel, {
          h: 160,
          label: modeParam.name.slice(0, 16),
          color: "#f2f2f2",
          binding: { kind: "param", node: p.id, paramId: modeParam.id, paramName: modeParam.name },
          options: { labels: Array.from({ length: modeParam.max + 1 }, (_, k) => String(k + 1)) },
        });
        widgets.push(sel);
      }
      const sw = makeWidget("footswitch", x + (modeParam ? 150 : 80), 210);
      Object.assign(sw, { label: "ON / OFF", color: "#ff5d5d", binding: { kind: "bypass", node: p.id } });
      widgets.push(sw);
      x += width + 30;
    }
    // One "MIXER" pedal per mixer node, with a volume fader for each input.
    for (const m of studio.nodes.filter((n) => n.kind === "mixer" && n.strips.length > 0)) {
      const strips = m.strips.slice(0, 6);
      const width = Math.max(160, 30 + strips.length * 80);
      const panel = makeWidget("panel", x, 30);
      Object.assign(panel, { w: width, h: 350, color: ENCLOSURES[6], label: "MIXER" });
      widgets.push(panel);
      strips.forEach((st, k) => {
        const f = makeWidget("fader", x + 20 + k * 80, 50);
        Object.assign(f, {
          w: 70,
          h: 240,
          label: nodeName(studio.nodes, st.source).slice(0, 10),
          color: "#7aa2ff",
          binding: { kind: "mix", node: m.id, source: st.source, field: "gainDb" },
        });
        widgets.push(f);
      });
      x += width + 30;
    }
    const tuner = makeWidget("tuner", x + 10, 40);
    const tap = makeWidget("footswitch", x + 80, 220);
    Object.assign(tap, { label: "TAP", color: "#ffa630", binding: { kind: "tap" } });
    const meter = makeWidget("meter", x + 270, 40);
    Object.assign(meter, { h: 300, label: "OUT" });
    widgets.push(tuner, tap, meter);
    const width = Math.max(1200, x + 350);
    commit({ ...board, width, height: Math.max(board.height, 420), widgets });
    setSelectedId(null);
    actions.toast(`Built a board for ${plugins.length} plugin${plugins.length === 1 ? "" : "s"}`, "success");
  };

  return (
    <RuntimeContext.Provider value={runtime}>
      <div className={`pedalboard ${editing ? "editing" : "playing"} ${stage ? "on-stage" : ""}`}>
        {stage && <StageHud onEdit={() => setMode("edit")} onRig={() => actions.setView("rig")} />}
        {!stage && (
          <div className="pb-toolbar">
            <div className="segmented small mode">
              <button className={!editing ? "on" : ""} onClick={() => (setMode("play"), setSelectedId(null))}>
                <Play size={13} /> Play
              </button>
              <button className={editing ? "on" : ""} onClick={() => setMode("edit")}>
                <Pencil size={13} /> Edit
              </button>
            </div>
            {editing && (
              <div className="pb-palette">
                {PALETTE.map((p) => (
                  <button key={p.type} className="btn small" onClick={() => add(p.type)} title={`Add ${WIDGET_META[p.type].name.toLowerCase()}`}>
                    {p.icon}
                    {WIDGET_META[p.type].name}
                  </button>
                ))}
              </div>
            )}
            <span className="spacer" />
            <span className="muted pb-title">{presets.current ?? "Untitled rig"} · pedalboard</span>
          </div>
        )}

        <div className="pb-body">
          <div
            className="pb-stage"
            ref={stageRef}
            onPointerDown={() => editing && setSelectedId(null)}
            data-tauri-drag-region={stage || undefined}
          >
            <div className="pb-board-frame" style={{ width: board.width * scale, height: board.height * scale }}>
              <div
                className={`pb-board surface-${board.surface} ${editing ? "grid" : ""}`}
                style={{ width: board.width, height: board.height, transform: `scale(${scale})` }}
                data-tauri-drag-region={stage || undefined}
              >
                {drawOrder(board.widgets).map((w) => (
                  <div
                    key={w.id}
                    className={`pb-widget type-${w.type} ${selectedId === w.id ? "selected" : ""}`}
                    style={{ left: w.x, top: w.y, width: w.w, height: w.h }}
                  >
                    <WidgetView w={w} editing={editing} />
                    {editing && (
                      <>
                        <div
                          className="pb-edit-overlay"
                          onPointerDown={(e) => onWidgetDown(e, w, "move")}
                          onPointerMove={onWidgetMove}
                          onPointerUp={onWidgetUp}
                        />
                        <div
                          className="pb-resize"
                          onPointerDown={(e) => onWidgetDown(e, w, "resize")}
                          onPointerMove={onWidgetMove}
                          onPointerUp={onWidgetUp}
                        />
                      </>
                    )}
                  </div>
                ))}
              </div>
            </div>

            {board.widgets.length === 0 && (
              <div className="pb-empty">
                <h3>Design your pedalboard</h3>
                <p>Lay out footswitches, knobs, faders and meters, and wire them to your plugins. Each preset keeps its own board.</p>
                <div className="row gap">
                  <button className="btn primary" onClick={generate} disabled={!studio.nodes.some((n) => n.kind === "plugin")}>
                    <WandSparkles size={14} /> Build from rig
                  </button>
                  {!editing && (
                    <button className="btn" onClick={() => setMode("edit")}>
                      <Pencil size={14} /> Start from scratch
                    </button>
                  )}
                </div>
                {!studio.nodes.some((n) => n.kind === "plugin") && (
                  <p className="muted small">Add plugins in the Rig view first to build a board from them.</p>
                )}
              </div>
            )}
          </div>

          {editing && (
            <Inspector
              board={board}
              selected={selected}
              onChange={updateWidget}
              onDelete={remove}
              onDuplicate={duplicate}
              onBoardChange={commit}
              onGenerate={generate}
            />
          )}
        </div>
      </div>
    </RuntimeContext.Provider>
  );
}
