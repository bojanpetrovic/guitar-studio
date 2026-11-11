import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ReactFlowProvider } from "@xyflow/react";
import {
  api,
  Catalog,
  nextFrame,
  pluginKey,
  PRESET_EXISTS,
  PresetList,
  PresetLoad,
  StudioView,
  SyncState,
} from "./api";
import { IS_REMOTE, lastEdit, on } from "./transport";
import { ActionsContext, AudioContext, CatalogContext, Ctx, Pos, StudioActions, StudioContext, View } from "./studio";
import { clampBpm } from "./util";
import TitleBar from "./components/TitleBar";
import PluginBrowser from "./components/PluginBrowser";
import PresetBrowser from "./components/PresetBrowser";
import GraphCanvas from "./components/GraphCanvas";
import Dock from "./components/Dock";
import SettingsDialog from "./components/SettingsDialog";
import Toasts, { Toast } from "./components/Toasts";
import PedalboardView from "./pedalboard/PedalboardView";
import { recoverFrame } from "./stageWindow";
import RemoteShell, { RemoteBoot } from "./remote/RemoteShell";
import ReplaceDialog from "./components/ReplaceDialog";
import { parseBoard } from "./pedalboard/types";
import { useSidebarOpen } from "./sidebar";

export default function App() {
  const [studio, setStudio] = useState<StudioView | null>(null);
  const [catalog, setCatalog] = useState<Catalog>({ plugins: [], errors: [], scannedAt: null, scanning: false });
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [replacing, setReplacing] = useState<number | null>(null);
  const [presets, setPresets] = useState<PresetList>({ presets: [], current: null, last: null, dir: "" });
  const [layoutKey, setLayoutKey] = useState(0);
  const [view, setView] = useState<View>("rig");
  const [stage, setStage] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const taps = useRef<number[]>([]);
  const toastId = useRef(1);
  const studioRef = useRef<StudioView | null>(null);
  studioRef.current = studio;
  /** Take a fresh engine view but keep our pedalboard (the UI owns it between preset loads).
   *  A phone can't edit the board, so it always takes the engine's. */
  const applyView = useCallback(
    (v: StudioView) => setStudio((s) => (s && !IS_REMOTE ? { ...v, pedalboard: s.pedalboard } : v)),
    [],
  );
  /** Rig generation we're showing (see SyncState). */
  const rig = useRef<number | null>(null);
  const presetsRef = useRef(presets);
  presetsRef.current = presets;

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const toast = useCallback(
    (message: string, kind: Toast["kind"] = "info") => {
      const id = toastId.current++;
      setToasts((t) => [...t.slice(-3), { id, message, kind }]);
      if (kind !== "loading") setTimeout(() => dismiss(id), kind === "error" ? 6000 : 3000);
      return id;
    },
    [dismiss],
  );

  const guard = useCallback(
    async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
      try {
        return await fn();
      } catch (e) {
        toast(String(e), "error");
        return undefined;
      }
    },
    [toast],
  );

  /** Apply a whole-rig swap (preset load / new). */
  const applyLoad = useCallback(
    (res: PresetLoad) => {
      rig.current = res.rig;
      setStudio(res.studio);
      setPresets(res.presets);
      setLayoutKey((k) => k + 1);
      res.warnings.forEach((w) => toast(w, "error"));
    },
    [toast],
  );

  // Preset loads run one at a time; asking again while one runs queues only the latest
  // (holding → or tapping it quickly doesn't stack up full rig swaps).
  const presetQueue = useRef<{ busy: boolean; next: string | null; loaded: string | null }>({
    busy: false,
    next: null,
    loaded: null,
  });
  const loadPresetNow = useCallback(
    async (name: string) => {
      const q = presetQueue.current;
      q.next = name;
      if (q.busy) return;
      q.busy = true;
      while (q.next) {
        const target = q.next;
        q.next = null;
        if (target === q.loaded && target === presetsRef.current.current) continue;
        // Instant when every plugin it needs is already running or kept warm.
        const ready = await api.presetReady(target).catch(() => false);
        const loading = ready ? null : toast(`Loading “${target}”…`, "loading");
        if (!ready) await nextFrame(); // let the toast paint: plugins load on the main thread
        try {
          applyLoad(await api.loadPreset(target));
          q.loaded = target;
        } catch (e) {
          toast(String(e), "error");
        } finally {
          if (loading !== null) dismiss(loading);
        }
      }
      q.busy = false;
    },
    [toast, dismiss, applyLoad],
  );
  /** Previous / next preset, counting from the one being loaded (so quick presses add up). */
  const stepPreset = useCallback(
    (dir: 1 | -1) => {
      const { presets: list, current } = presetsRef.current;
      const q = presetQueue.current;
      const names = list.map((p) => p.name);
      if (names.length === 0) return;
      const from = q.next ?? (q.busy ? q.loaded : null) ?? current;
      const at = from ? names.indexOf(from) : -1;
      if (names.length < 2 && at >= 0) return;
      const target = names[(at + dir + names.length) % names.length];
      if (target !== from) loadPresetNow(target);
    },
    [loadPresetNow],
  );

  // html/body must go see-through too while the board is on stage.
  useEffect(() => {
    document.documentElement.classList.toggle("stage-mode", stage);
  }, [stage]);

  /** The other screen changed something: mirror it. A different rig generation means a
   *  whole new rig (re-fit, fresh preset list). */
  const applySync = useCallback(
    (sync: SyncState) => {
      const swapped = rig.current !== null && sync.rig !== rig.current;
      rig.current = sync.rig;
      if (swapped) {
        setStudio(sync.studio);
        setLayoutKey((k) => k + 1);
      } else applyView(sync.studio);
      if (swapped || sync.preset !== presetsRef.current.current) api.listPresets().then(setPresets, () => {});
    },
    [applyView],
  );

  // Pushes arrive while this window may be mid-gesture (a fader drag): hold them until
  // our own edits settle, then read the engine fresh (the held push may predate them).
  useEffect(() => {
    const QUIET_MS = 400;
    let timer: number | undefined;
    const settle = () => {
      const wait = lastEdit() + QUIET_MS - performance.now();
      if (wait > 0) timer = window.setTimeout(settle, wait + 10);
      else api.remoteSync().then(applySync, () => {});
    };
    const un = on<SyncState>("sync", (sync) => {
      if (performance.now() - lastEdit() >= QUIET_MS) applySync(sync);
      else {
        window.clearTimeout(timer);
        settle();
      }
    });
    // A phone that reconnects may have missed pushes.
    const unRe = on("reconnected", () => {
      api.remoteSync().then(applySync, () => {});
      api.listPresets().then(setPresets, () => {});
    });
    return () => {
      window.clearTimeout(timer);
      un();
      unRe();
    };
  }, [applySync]);

  const launched = useRef(false);
  useEffect(() => {
    if (launched.current) return; // StrictMode runs effects twice in dev
    launched.current = true;
    if (IS_REMOTE) {
      Promise.all([api.remoteSync(), api.listPresets()]).then(([sync, p]) => {
        rig.current = sync.rig;
        setStudio(sync.studio);
        setPresets(p);
      }, () => {});
      return;
    }
    recoverFrame();
    // On launch, bring back the rig that was active last time (only into a blank studio,
    // so a dev hot-reload doesn't clobber work in progress).
    Promise.all([api.remoteSync(), api.listPresets()]).then(([sync, p]) => {
      const s = sync.studio;
      rig.current = sync.rig;
      setStudio(s);
      setPresets(p);
      const blank = s.nodes.length <= 2;
      if (blank && !p.current && p.last && p.presets.some((x) => x.name === p.last)) loadPresetNow(p.last);
    }, (e) => setBootError(String(e)));
    api.getCatalog().then(setCatalog, () => {});
    on<Catalog>("catalog", setCatalog);
  }, []);

  /** Wire a freshly added node into the main chain, just before the Output. */
  const appendToChain = useCallback(
    async (id: number, view: StudioView) => {
      const out = view.nodes.find((n) => n.kind === "output");
      const feed = out && view.edges.find((e) => e.to === out.id && e.from !== id);
      if (!out || !feed) return view;
      try {
        await api.disconnect(feed.from, out.id);
        await api.connect(feed.from, id);
        return await api.connect(id, out.id);
      } catch (e) {
        // Show the node anyway, with whatever cables the engine ended up with.
        toast(String(e), "error");
        return api.getStudio().catch(() => view);
      }
    },
    [toast],
  );

  /** Move a node (the canvas has already drawn it there). */
  const moveNode = useCallback((id: number, x: number, y: number) => {
    setStudio((s) => s && { ...s, nodes: s.nodes.map((n) => (n.id === id ? { ...n, x, y } : n)) });
    api.moveNode(id, x, y).catch(() => {});
  }, []);

  /** Place a new chain node between the last chain node and the Output, shifting Output right. */
  const chainPosition = useCallback((): Pos => {
    const v = studioRef.current;
    const out = v?.nodes.find((n) => n.kind === "output");
    const feed = out && v?.edges.find((e) => e.to === out.id);
    const prev = feed && v?.nodes.find((n) => n.id === feed.from);
    if (!out || !prev) return { x: 400, y: 240 };
    const x = prev.x + 300;
    if (out.x < x + 300) moveNode(out.id, x + 300, out.y);
    return { x, y: prev.y };
  }, [moveNode]);

  const actions: StudioActions = useMemo(
    () => ({
      apply: applyView,
      toast,
      openSettings: () => setSettingsOpen(true),
      addPlugin: async (p, pos) => {
        const loading = toast(`Loading ${p.name}…`, "loading");
        await nextFrame();
        const autoWire = !pos;
        const at = pos ?? chainPosition();
        const res = await guard(() => api.addPluginNode(pluginKey(p), at.x, at.y));
        dismiss(loading);
        if (!res) return;
        const [id, view] = res;
        applyView(autoWire ? await appendToChain(id, view) : view);
      },
      addMixer: async (pos) => {
        const autoWire = !pos;
        const at = pos ?? chainPosition();
        const res = await guard(() => api.addMixerNode(at.x, at.y));
        if (!res) return;
        const [id, view] = res;
        applyView(autoWire ? await appendToChain(id, view) : view);
      },
      setMixerStrip: (id, source, field, value) => {
        setStudio(
          (s) =>
            s && {
              ...s,
              nodes: s.nodes.map((n) =>
                n.id === id
                  ? {
                      ...n,
                      strips: n.strips.map((st) =>
                        st.source === source ? { ...st, [field]: field === "mute" ? value >= 0.5 : value } : st,
                      ),
                    }
                  : n,
              ),
            },
        );
        api.setMixerStrip(id, source, field, value).catch((e) => toast(String(e), "error"));
      },
      addGain: async (pos) => {
        const autoWire = !pos;
        const at = pos ?? chainPosition();
        const res = await guard(() => api.addGainNode(at.x, at.y));
        if (!res) return;
        const [id, view] = res;
        applyView(autoWire ? await appendToChain(id, view) : view);
      },
      pickReplacement: setReplacing,
      replacePlugin: async (id, p) => {
        const before = studioRef.current;
        const old = before?.nodes.find((n) => n.id === id)?.plugin;
        if (!before || !old || pluginKey(old) === pluginKey(p)) return;
        const loading = toast(`Loading ${p.name}…`, "loading");
        await nextFrame(); // plugins load on the main thread
        const v = await guard(() => api.replacePlugin(id, pluginKey(p)));
        dismiss(loading);
        if (!v) return;
        applyView(v);
        // Board controls on the old plugin's parameters: move them to the new plugin's
        // parameter of the same name, where there is one.
        const board = parseBoard(before.pedalboard);
        const bound = board.widgets.filter((w) => w.binding.kind === "param" && w.binding.node === id);
        let lost = 0;
        if (bound.length) {
          const params = await api.pluginParams(id).catch(() => []);
          const byName = new Map(params.map((q) => [q.name.trim().toLowerCase(), q]));
          const widgets = board.widgets.map((w) => {
            if (w.binding.kind !== "param" || w.binding.node !== id) return w;
            const q = byName.get(w.binding.paramName.trim().toLowerCase());
            if (!q) {
              lost++;
              return w;
            }
            return { ...w, binding: { ...w.binding, paramId: q.id, paramName: q.name } };
          });
          setStudio((s) => s && { ...s, pedalboard: { ...board, widgets } });
          api.setPedalboard({ ...board, widgets }).catch((e) => toast(String(e), "error"));
        }
        toast(`Replaced ${old.name} with ${p.name}`, "success");
        if (lost) toast(`${lost} pedalboard control${lost === 1 ? "" : "s"} for ${old.name} need a new parameter`, "info");
      },
      remove: async (id) => {
        const v = await guard(() => api.removeNode(id));
        if (v) applyView(v);
      },
      connect: async (from, to) => {
        const v = await guard(() => api.connect(from, to));
        if (v) applyView(v);
      },
      disconnect: async (from, to) => {
        const v = await guard(() => api.disconnect(from, to));
        if (v) applyView(v);
      },
      setParam: (id, param, value) => {
        setStudio(
          (s) => s && { ...s, nodes: s.nodes.map((n) => (n.id === id ? { ...n, [param]: value } : n)) },
        );
        api.setNodeParam(id, param, value).catch((e) => toast(String(e), "error"));
      },
      setBypass: async (id, bypass) => {
        try {
          await api.setBypass(id, bypass);
          setStudio((s) => s && { ...s, nodes: s.nodes.map((n) => (n.id === id ? { ...n, bypass } : n)) });
        } catch (e) {
          toast(String(e), "error");
        }
      },
      moveNode,
      openEditor: async (id) => {
        await nextFrame();
        await guard(() => api.openEditor(id));
      },
      loadPreset: loadPresetNow,
      stepPreset,
      savePreset: async (name, overwrite = false) => {
        try {
          const [list, warnings] = await api.savePreset(name, overwrite);
          setPresets(list);
          warnings.forEach((w) => toast(w, "error"));
          toast(`Saved “${list.current}”`, "success");
          return "ok";
        } catch (e) {
          if (String(e) === PRESET_EXISTS) return "exists";
          toast(String(e), "error");
          return "error";
        }
      },
      newPreset: async () => {
        const res = await guard(() => api.newPreset());
        if (res) applyLoad(res);
      },
      deletePreset: async (name) => {
        const list = await guard(() => api.deletePreset(name));
        if (list) {
          setPresets(list);
          toast(`Deleted “${name}”`, "info");
        }
      },
      renamePreset: async (from, to) => {
        const list = await guard(() => api.renamePreset(from, to));
        if (list) setPresets(list);
        return !!list;
      },
      setMetronome: (patch) => {
        const cur = studioRef.current;
        if (!cur) return;
        const next = { ...cur.metronome, ...patch };
        next.bpm = clampBpm(next.bpm);
        setStudio((s) => s && { ...s, metronome: next });
        api.setMetronome(next).catch((e) => toast(String(e), "error"));
      },
      tapTempo: () => {
        const now = performance.now();
        taps.current = [...taps.current.filter((t) => now - t < 2500), now].slice(-6);
        if (taps.current.length < 2) return;
        const gaps = taps.current.slice(1).map((t, i) => t - taps.current[i]);
        const bpm = 60000 / (gaps.reduce((a, b) => a + b, 0) / gaps.length);
        const cur = studioRef.current;
        if (!cur) return;
        const next = { ...cur.metronome, bpm: clampBpm(bpm) };
        setStudio((s) => s && { ...s, metronome: next });
        api.setMetronome(next).catch(() => {});
      },
      setTuner: (enabled, referenceA4) => {
        setStudio((s) => s && { ...s, tunerEnabled: enabled, referenceA4: referenceA4 ?? s.referenceA4 });
        api.setTuner(enabled, referenceA4 ?? studioRef.current?.referenceA4).catch((e) => toast(String(e), "error"));
      },
      updateBoard: (board) => {
        setStudio((s) => s && { ...s, pedalboard: board });
        api.setPedalboard(board).catch((e) => toast(String(e), "error"));
      },
      setView,
      setStage,
      addScene: async () => {
        const v = await guard(() => api.addScene());
        if (v) applyView(v);
      },
      selectScene: async (index) => {
        const v = await guard(() => api.selectScene(index));
        if (v) applyView(v);
      },
      renameScene: async (index, name) => {
        const v = await guard(() => api.renameScene(index, name));
        if (v) applyView(v);
        return !!v;
      },
      deleteScene: async (index) => {
        const v = await guard(() => api.deleteScene(index));
        if (v) applyView(v);
      },
      scan: async () => {
        const c = await guard(() => api.scanPlugins());
        if (c) {
          setCatalog(c);
          toast(`Found ${c.plugins.length} plugins`, "success");
        }
      },
    }),
    [toast, guard, dismiss, appendToChain, chainPosition, moveNode, loadPresetNow, stepPreset, applyLoad, applyView],
  );

  const closeSettings = useCallback(() => setSettingsOpen(false), []);
  const closeReplace = useCallback(() => setReplacing(null), []);
  const ctx: Ctx | null = useMemo(
    () =>
      studio && {
        studio,
        catalog,
        presets,
        layoutKey,
        view: IS_REMOTE ? "board" : view,
        stage: !IS_REMOTE && stage,
        remote: IS_REMOTE,
        actions,
      },
    [studio, catalog, presets, layoutKey, view, stage, actions],
  );

  if (!studio || !ctx) {
    if (IS_REMOTE) return <RemoteBoot />;
    return <div className="boot">{bootError ? `The audio engine didn't start: ${bootError}` : "Starting audio engine…"}</div>;
  }

  return (
    <StudioContext.Provider value={ctx}>
      <ActionsContext.Provider value={actions}>
        <CatalogContext.Provider value={catalog}>
          <AudioContext.Provider value={studio.audio}>
            {IS_REMOTE ? (
              <RemoteShell />
            ) : (
              <ReactFlowProvider>
                <Desktop view={view} stage={stage} />
              </ReactFlowProvider>
            )}
            {settingsOpen && <SettingsDialog onClose={closeSettings} />}
            {replacing !== null && studio.nodes.some((n) => n.id === replacing && n.plugin) && (
              <ReplaceDialog node={studio.nodes.find((n) => n.id === replacing)!} onClose={closeReplace} />
            )}
            <Toasts toasts={toasts} onDismiss={dismiss} />
          </AudioContext.Provider>
        </CatalogContext.Provider>
      </ActionsContext.Provider>
    </StudioContext.Provider>
  );
}

/** The desktop window: title bar, presets (left), rig or pedalboard, plugins (right), and the dock. */
function Desktop({ view, stage }: { view: View; stage: boolean }) {
  const left = useSidebarOpen("left");
  const right = useSidebarOpen("right");
  return (
    <div className={`app view-${view} ${stage ? "stage-mode" : ""} ${left ? "" : "left-closed"} ${right ? "" : "right-closed"}`}>
      <TitleBar />
      <aside className="sidebar left">
        <PresetBrowser />
      </aside>
      <main className="stage">{view === "rig" ? <GraphCanvas /> : <PedalboardView />}</main>
      <aside className="sidebar right">
        <PluginBrowser />
      </aside>
      <Dock />
    </div>
  );
}
