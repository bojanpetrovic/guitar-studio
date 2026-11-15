import { keys } from "../platform";
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
import { usePresetStep, useStudio } from "../studio";
import { isTyping } from "../util";
import { useLink } from "../transport";
import { api, nodeName, NodeView, ParamInfo } from "../api";
import Inspector from "./Inspector";
import Surface from "./Surface";
import { RuntimeContext, useParamRuntime } from "./runtime";
import { clearKeySelection, WidgetView } from "./widgets";
import { enterStage, exitStage } from "../stageWindow";
import SceneBar, { SCENE_BAR_H } from "../components/SceneBar";
import {
  ACCENTS,
  Board,
  drawOrder,
  ENCLOSURES,
  fitBoard,
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

/** A move drags every selected widget (`orig`, as they were); a resize just `id`.
 *  `collapse`: a plain click on a widget of a group selects just it, if nothing moved. */
type Drag = { id: string; kind: "move" | "resize"; x: number; y: number; orig: Widget[]; moved: boolean; collapse: boolean };

/** A selection box being dragged out on the board, in board units. */
type Marquee = { x0: number; y0: number; x1: number; y1: number; base: string[]; moved: boolean };

/** What ⌘C / ⌘X took, kept across presets so controls can be pasted onto another
 *  board. `pastes` offsets each paste a step further, so copies don't stack up. */
const clipboard: { widgets: Widget[]; pastes: number } = { widgets: [], pastes: 0 };

/** How far `ws` can move by (dx, dy) and stay on the board, as a group. */
function clampGroup(ws: Widget[], dx: number, dy: number, board: Board) {
  const minX = Math.min(...ws.map((w) => w.x));
  const minY = Math.min(...ws.map((w) => w.y));
  const maxX = Math.max(...ws.map((w) => w.x + w.w));
  const maxY = Math.max(...ws.map((w) => w.y + w.h));
  return {
    dx: Math.max(-minX, Math.min(board.width - maxX, dx)),
    dy: Math.max(-minY, Math.min(board.height - maxY, dy)),
  };
}

/** Copies of `ws` with new ids, moved by (dx, dy) as far as the board allows. */
function copiesOf(ws: Widget[], dx: number, dy: number, board: Board): Widget[] {
  const d = clampGroup(ws, dx, dy, board);
  return ws.map((w) => ({ ...w, id: newId(), x: w.x + d.dx, y: w.y + d.dy, binding: { ...w.binding }, options: { ...w.options } }));
}

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
  const { presets } = useStudio();
  const [awake, setAwake] = useState(true);
  const hovering = useRef(false);
  const timer = useRef<number | undefined>(undefined);
  const { step, count } = usePresetStep();

  // Awake on mount and on mouse movement; asleep 2.2 s later unless hovered.
  useEffect(() => {
    const wake = () => {
      setAwake(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => !hovering.current && setAwake(false), 2200);
    };
    wake();
    window.addEventListener("pointermove", wake);
    return () => {
      window.clearTimeout(timer.current);
      window.removeEventListener("pointermove", wake);
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.repeat || isTyping(e)) return;
      if (e.key === "ArrowLeft" || e.key === "PageUp" || e.key === "[") step(-1);
      else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === "]") step(1);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step]);

  return (
    <div
      className={`stage-hud ${awake ? "awake" : ""}`}
      onPointerEnter={() => (hovering.current = true)}
      onPointerLeave={() => (hovering.current = false)}
    >
      <button className="icon-btn ghost" onClick={() => step(-1)} disabled={count === 0} title="Previous preset (←)">
        <ChevronLeft size={16} />
      </button>
      <span className="stage-hud-name" data-tauri-drag-region>
        {presets.current ?? "Untitled rig"}
      </span>
      <button className="icon-btn ghost" onClick={() => step(1)} disabled={count === 0} title="Next preset (→)">
        <ChevronRight size={16} />
      </button>
      <span className="stage-hud-sep" />
      <button className="btn small" onClick={onEdit} title={keys("Edit the board (⌘E or Esc)")}>
        <Pencil size={13} /> Edit
      </button>
      <button className="btn small" onClick={onRig} title={keys("Back to the rig (⌘1)")}>
        <Workflow size={13} /> Rig
      </button>
    </div>
  );
}

/** Height the floating tuner/metronome dock takes under the board on stage (see .dock in styles.css). */
const STAGE_DOCK_H = 64;

export default function PedalboardView() {
  const { studio, presets, layoutKey, remote, actions } = useStudio();
  const stored = useMemo(() => parseBoard(studio.pedalboard), [studio.pedalboard]);
  const [draft, setDraft] = useState<Board | null>(null);
  const board = draft ?? stored;
  const [mode, setMode] = useState<"play" | "edit">(remote || stored.widgets.length ? "play" : "edit");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  // A phone pauses polling while its link is down (requests would only pile up).
  const link = useLink();
  const runtime = useParamRuntime(board, studio, link === "open");
  const editing = mode === "edit";
  // A key-selected control stays selected only while this board is being played.
  useEffect(() => clearKeySelection, [editing, layoutKey]);
  const selection = useMemo(() => board.widgets.filter((w) => selectedIds.includes(w.id)), [board.widgets, selectedIds]);
  const isSelected = (id: string) => selectedIds.includes(id);
  // The inspector edits one control at a time.
  const selected = selection.length === 1 ? selection[0] : null;

  // A freshly loaded rig opens ready to play if it has a board, ready to design if not.
  useEffect(() => {
    if (!layoutKey) return;
    setMode(remote || parseBoard(studio.pedalboard).widgets.length ? "play" : "edit");
    setSelectedIds([]);
    setDraft(null);
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
  /** The board with each of `changed` swapped in for its namesake. */
  const withWidgets = useCallback(
    (changed: Widget[]) => {
      const byId = new Map(changed.map((w) => [w.id, w]));
      return { ...board, widgets: board.widgets.map((x) => byId.get(x.id) ?? x) };
    },
    [board],
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
  // (A phone just plays: no window to manage, its own header instead of the HUD.)
  const stage = !remote && !editing && board.widgets.length > 0;
  const bare = stage || remote;
  useEffect(() => {
    if (!stage) return;
    actions.setStage(true);
    return () => {
      actions.setStage(false);
      exitStage();
    };
  }, [stage, actions]);
  const showScenes = board.showScenes !== false;
  useEffect(() => {
    if (stage) enterStage(board.width / board.height, STAGE_DOCK_H + (showScenes ? SCENE_BAR_H : 0));
  }, [stage, board.width, board.height, showScenes]);

  // ⌘E flips between playing and designing; Esc leaves the stage for the editor.
  useEffect(() => {
    if (remote) return;
    const onKey = (e: KeyboardEvent) => {
      const typing = isTyping(e);
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "e") {
        e.preventDefault();
        setMode((m) => (m === "edit" ? "play" : "edit"));
        setSelectedIds([]);
      } else if (stage && !typing && e.key === "Escape" && !e.defaultPrevented) {
        setMode("edit");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stage, remote]);

  const margin = stage ? 0 : remote ? 12 : 48;
  const scale = Math.max(0.1, Math.min((avail.w - margin) / board.width, (avail.h - margin) / board.height, bare ? 4 : 1.4));

  // ---- editing gestures
  const drag = useRef<Drag | null>(null);
  const onWidgetDown = (e: React.PointerEvent, w: Widget, kind: Drag["kind"]) => {
    if (!editing) return;
    e.stopPropagation();
    let ids = [w.id];
    if (kind === "move" && (e.shiftKey || e.metaKey || e.ctrlKey)) {
      // ⇧/⌘-click adds to the selection, or takes a widget back out of it.
      if (isSelected(w.id)) {
        setSelectedIds(selectedIds.filter((id) => id !== w.id));
        return;
      }
      ids = [...selectedIds, w.id];
    } else if (kind === "move" && isSelected(w.id)) {
      ids = selectedIds; // grabbing one of a group moves the group
    }
    setSelectedIds(ids);
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    const orig = kind === "move" ? board.widgets.filter((x) => ids.includes(x.id)) : [w];
    drag.current = { id: w.id, kind, x: e.clientX, y: e.clientY, orig, moved: false, collapse: ids.length > 1 && ids === selectedIds };
  };
  const onWidgetMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const dx = (e.clientX - d.x) / scale;
    const dy = (e.clientY - d.y) / scale;
    if (!d.moved && Math.hypot(dx, dy) < 2) return;
    d.moved = true;
    if (d.kind === "move") {
      // The grabbed widget snaps to the grid; the rest keep their places relative to it.
      const grabbed = d.orig.find((w) => w.id === d.id) ?? d.orig[0];
      const by = clampGroup(d.orig, snap(grabbed.x + dx) - grabbed.x, snap(grabbed.y + dy) - grabbed.y, board);
      setDraft(withWidgets(d.orig.map((w) => ({ ...w, x: w.x + by.dx, y: w.y + by.dy }))));
    } else {
      const w = d.orig[0];
      const m = WIDGET_META[w.type];
      setDraft(
        withWidgets([
          {
            ...w,
            w: Math.max(m.minW, Math.min(board.width - w.x, snap(w.w + dx))),
            h: Math.max(m.minH, Math.min(board.height - w.y, snap(w.h + dy))),
          },
        ]),
      );
    }
  };
  // A cancelled gesture (lost capture, OS gesture) ends the drag like a release.
  const onWidgetUp = () => {
    const d = drag.current;
    if (d?.moved && draft) commit(draft);
    else if (d?.collapse) setSelectedIds([d.id]);
    drag.current = null;
  };

  // ---- selection box: drag on an empty part of the board to select what it touches
  // (a pedal enclosure only when it's wholly inside, or it would catch everything on it).
  const boardRef = useRef<HTMLDivElement>(null);
  const marqueeRef = useRef<Marquee | null>(null);
  const [marquee, setMarquee] = useState<Marquee | null>(null);
  const toBoard = (e: React.PointerEvent) => {
    const r = boardRef.current!.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(board.width, (e.clientX - r.left) / scale)),
      y: Math.max(0, Math.min(board.height, (e.clientY - r.top) / scale)),
    };
  };
  const onStageDown = (e: React.PointerEvent) => {
    if (!editing || e.button !== 0 || !boardRef.current) return;
    // Not from the "Design your pedalboard" card: capturing would swallow its buttons' clicks.
    if ((e.target as Element).closest(".pb-empty")) return;
    const p = toBoard(e);
    const add = e.shiftKey || e.metaKey || e.ctrlKey;
    marqueeRef.current = { x0: p.x, y0: p.y, x1: p.x, y1: p.y, base: add ? selectedIds : [], moved: false };
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    if (!add) setSelectedIds([]);
  };
  const onStageMove = (e: React.PointerEvent) => {
    const m = marqueeRef.current;
    if (!m) return;
    const p = toBoard(e);
    const next = { ...m, x1: p.x, y1: p.y, moved: m.moved || Math.hypot(p.x - m.x0, p.y - m.y0) * scale > 3 };
    marqueeRef.current = next;
    if (!next.moved) return;
    setMarquee(next);
    const [l, r] = [Math.min(next.x0, next.x1), Math.max(next.x0, next.x1)];
    const [t, b] = [Math.min(next.y0, next.y1), Math.max(next.y0, next.y1)];
    const hit = board.widgets
      .filter((w) =>
        w.type === "panel"
          ? w.x >= l && w.x + w.w <= r && w.y >= t && w.y + w.h <= b
          : w.x < r && w.x + w.w > l && w.y < b && w.y + w.h > t,
      )
      .map((w) => w.id);
    setSelectedIds([...new Set([...next.base, ...hit])]);
  };
  const onStageUp = () => {
    marqueeRef.current = null;
    setMarquee(null);
  };

  const add = (type: WidgetType) => {
    const m = WIDGET_META[type];
    const n = board.widgets.length;
    const w = makeWidget(type, board.width / 2 - m.w / 2 + (n % 5) * GRID * 2, board.height / 2 - m.h / 2 + (n % 5) * GRID * 2);
    w.color = type === "panel" ? ENCLOSURES[n % ENCLOSURES.length] : ACCENTS[0];
    commit({ ...board, widgets: [...board.widgets, w] });
    setSelectedIds([w.id]);
  };

  const remove = useCallback(() => {
    if (!selectedIds.length) return;
    commit({ ...board, widgets: board.widgets.filter((w) => !selectedIds.includes(w.id)) });
    setSelectedIds([]);
  }, [board, selectedIds, commit]);

  /** Adds `copies` to the board and selects them. */
  const place = useCallback(
    (copies: Widget[]) => {
      commit({ ...board, widgets: [...board.widgets, ...copies] });
      setSelectedIds(copies.map((w) => w.id));
    },
    [board, commit],
  );
  const duplicate = useCallback(() => {
    if (selection.length) place(copiesOf(selection, GRID * 2, GRID * 2, board));
  }, [board, selection, place]);
  const copy = useCallback(() => {
    if (!selection.length) return false;
    clipboard.widgets = selection.map((w) => ({ ...w }));
    clipboard.pastes = 0;
    return true;
  }, [selection]);
  const paste = useCallback(() => {
    if (!clipboard.widgets.length) return;
    // Onto another board: where they were. Next to the originals otherwise.
    const here = clipboard.widgets.some((w) => board.widgets.some((x) => x.id === w.id));
    const step = GRID * 2 * (++clipboard.pastes - (here ? 0 : 1));
    place(copiesOf(clipboard.widgets, step, step, board));
  }, [board, place]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e)) return;
      const cmd = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      if (cmd && key === "a") {
        e.preventDefault();
        setSelectedIds(board.widgets.map((w) => w.id));
        return;
      }
      if (cmd && key === "v") {
        e.preventDefault();
        paste();
        return;
      }
      if (!selection.length) return;
      if (e.key === "Backspace" || e.key === "Delete") {
        e.preventDefault();
        remove();
      } else if (cmd && key === "d") {
        e.preventDefault();
        duplicate();
      } else if (cmd && (key === "c" || key === "x")) {
        // Leave copying selected text (outside the board) to the browser.
        if (!window.getSelection()?.isCollapsed) return;
        e.preventDefault();
        if (copy() && key === "x") remove();
      } else if (e.key.startsWith("Arrow")) {
        e.preventDefault();
        const step = e.shiftKey ? GRID * 5 : GRID;
        const by = clampGroup(
          selection,
          e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0,
          e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0,
          board,
        );
        commit(withWidgets(selection.map((w) => ({ ...w, x: w.x + by.dx, y: w.y + by.dy }))));
      } else if (e.key === "Escape") {
        setSelectedIds([]);
      }
    };
    // On document, so a selected widget claims the arrow keys before the window-level
    // tempo keys see them (those skip events that were already handled).
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [editing, selection, board, remove, duplicate, copy, paste, commit, withWidgets]);

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
      Object.assign(panel, {
        w: width,
        // Tall enough that the name (along the bottom edge) clears the footswitch.
        h: 390,
        color: ENCLOSURES[i % ENCLOSURES.length],
        label: (p.plugin?.name ?? "Pedal").toUpperCase().slice(0, 22),
        binding: { kind: "bypass", node: p.id },
      });
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
      const sw = makeWidget("footswitch", x + (modeParam ? 150 : 80), 190);
      Object.assign(sw, { label: "ON / OFF", color: "#ff5d5d", binding: { kind: "bypass", node: p.id } });
      widgets.push(sw);
      x += width + 30;
    }
    // One "MIXER" pedal per mixer node, with a volume fader for each input.
    for (const m of studio.nodes.filter((n) => n.kind === "mixer" && n.strips.length > 0)) {
      const strips = m.strips.slice(0, 6);
      const width = Math.max(160, 30 + strips.length * 80);
      const panel = makeWidget("panel", x, 30);
      Object.assign(panel, { w: width, h: 390, color: ENCLOSURES[6], label: "MIXER" });
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
    commit(fitBoard({ ...board, widgets }));
    setSelectedIds([]);
    actions.toast(`Built a board for ${plugins.length} plugin${plugins.length === 1 ? "" : "s"}`, "success");
  };

  return (
    <RuntimeContext.Provider value={runtime}>
      <div className={`pedalboard ${editing ? "editing" : "playing"} ${stage ? "on-stage" : ""} ${remote ? "on-phone" : ""}`}>
        {stage && <StageHud onEdit={() => setMode("edit")} onRig={() => actions.setView("rig")} />}
        {!bare && (
          <div className="pb-toolbar">
            <div className="segmented small mode">
              <button className={!editing ? "on" : ""} onClick={() => (setMode("play"), setSelectedIds([]))}>
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
            onPointerDown={onStageDown}
            onPointerMove={onStageMove}
            onPointerUp={onStageUp}
            onPointerCancel={onStageUp}
            data-tauri-drag-region={stage || undefined}
          >
            <div className="pb-board-frame" style={{ width: board.width * scale, height: board.height * scale }}>
              <div
                ref={boardRef}
                className={`pb-board surface-${board.surface} ${editing ? "grid" : ""}`}
                style={{ width: board.width, height: board.height, transform: `scale(${scale})` }}
                data-tauri-drag-region={stage || undefined}
              >
                {/* Animated surfaces draw here, under the controls. */}
                <Surface surface={board.surface} />
                {drawOrder(board.widgets).map((w) => (
                  <div
                    key={w.id}
                    className={`pb-widget type-${w.type} ${isSelected(w.id) ? "selected" : ""}`}
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
                          onPointerCancel={onWidgetUp}
                        />
                        <div
                          className="pb-resize"
                          onPointerDown={(e) => onWidgetDown(e, w, "resize")}
                          onPointerMove={onWidgetMove}
                          onPointerUp={onWidgetUp}
                          onPointerCancel={onWidgetUp}
                        />
                      </>
                    )}
                  </div>
                ))}
              </div>
              {marquee && (
                <div
                  className="pb-marquee"
                  style={{
                    left: Math.min(marquee.x0, marquee.x1) * scale,
                    top: Math.min(marquee.y0, marquee.y1) * scale,
                    width: Math.abs(marquee.x1 - marquee.x0) * scale,
                    height: Math.abs(marquee.y1 - marquee.y0) * scale,
                  }}
                />
              )}
            </div>

            {board.widgets.length === 0 && remote && (
              <div className="pb-empty">
                <h3>No pedalboard yet</h3>
                <p>This rig doesn’t have a board. Design one in Guitar Studio on your computer (Pedalboard → Edit) and it shows up here.</p>
              </div>
            )}
            {board.widgets.length === 0 && !remote && (
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
              count={selection.length}
              onChange={updateWidget}
              onDelete={remove}
              onDuplicate={duplicate}
              onBoardChange={commit}
              onGenerate={generate}
            />
          )}
        </div>
        {showScenes && <SceneBar variant="board" />}
      </div>
    </RuntimeContext.Provider>
  );
}
