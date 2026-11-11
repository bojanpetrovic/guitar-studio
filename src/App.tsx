import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { ReactFlowProvider } from "@xyflow/react";
import {
  api,
  Catalog,
  nextFrame,
  PluginDescriptor,
  MetronomeSettings,
  pluginKey,
  PRESET_EXISTS,
  PresetList,
  PresetLoad,
  StripField,
  StudioView,
  SyncState,
} from "./api";
import { IS_REMOTE, lastEdit, on } from "./transport";
import TitleBar from "./components/TitleBar";
import PluginBrowser from "./components/PluginBrowser";
import GraphCanvas from "./components/GraphCanvas";
import Dock from "./components/Dock";
import SettingsDialog from "./components/SettingsDialog";
import Toasts, { Toast } from "./components/Toasts";
import PedalboardView from "./pedalboard/PedalboardView";
import { recoverFrame } from "./stageWindow";
import RemoteShell, { RemoteBoot } from "./remote/RemoteShell";

type Pos = { x: number; y: number };

export interface StudioActions {
  addPlugin: (p: PluginDescriptor, pos?: Pos) => Promise<void>;
  addGain: (pos?: Pos) => Promise<void>;
  addMixer: (pos?: Pos) => Promise<void>;
  setMixerStrip: (id: number, source: number, field: StripField, value: number) => void;
  remove: (id: number) => Promise<void>;
  connect: (from: number, to: number) => Promise<void>;
  disconnect: (from: number, to: number) => Promise<void>;
  setParam: (id: number, param: "gainDb" | "channel", value: number) => void;
  setBypass: (id: number, bypass: boolean) => Promise<void>;
  openEditor: (id: number) => Promise<void>;
  scan: () => Promise<void>;
  apply: (v: StudioView) => void;
  openSettings: () => void;
  toast: (message: string, kind?: Toast["kind"]) => number;
  loadPreset: (name: string) => Promise<void>;
  /** Resolves to "exists" when the name belongs to another preset and overwrite is false. */
  savePreset: (name: string, overwrite?: boolean) => Promise<"ok" | "exists" | "error">;
  newPreset: () => Promise<void>;
  deletePreset: (name: string) => Promise<void>;
  renamePreset: (from: string, to: string) => Promise<boolean>;
  setMetronome: (patch: Partial<MetronomeSettings>) => void;
  tapTempo: () => void;
  setTuner: (enabled: boolean, referenceA4?: number) => void;
  /** Replace the pedalboard layout (kept locally and mirrored to the engine). */
  updateBoard: (board: unknown) => void;
  setView: (view: View) => void;
  /** The pedalboard is being played full-window (frameless); hide the studio chrome. */
  setStage: (on: boolean) => void;
  /** Scenes: snapshots of every plugin's settings. Adding copies the current sound. */
  addScene: () => Promise<void>;
  selectScene: (index: number) => Promise<void>;
  renameScene: (index: number, name: string) => Promise<boolean>;
  deleteScene: (index: number) => Promise<void>;
}

export type View = "rig" | "board";

interface Ctx {
  studio: StudioView;
  catalog: Catalog;
  presets: PresetList;
  /** Bumps whenever a whole rig is swapped in, so the canvas can re-fit. */
  layoutKey: number;
  view: View;
  stage: boolean;
  /** This UI is a phone remote-controlling the desktop app (play only). */
  remote: boolean;
  actions: StudioActions;
}

const StudioContext = createContext<Ctx | null>(null);
export const useStudio = () => {
  const ctx = useContext(StudioContext);
  if (!ctx) throw new Error("StudioContext missing");
  return ctx;
};

export default function App() {
  const [studio, setStudio] = useState<StudioView | null>(null);
  const [catalog, setCatalog] = useState<Catalog>({ plugins: [], errors: [], scannedAt: null, scanning: false });
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [presets, setPresets] = useState<PresetList>({ presets: [], current: null, last: null, dir: "" });
  const [layoutKey, setLayoutKey] = useState(0);
  const [view, setView] = useState<View>("rig");
  const [stage, setStage] = useState(false);
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

  const loadPresetNow = useCallback(
    async (name: string) => {
      const loading = toast(`Loading “${name}”…`, "loading");
      await nextFrame(); // let the toast paint: plugins load on the main thread
      try {
        applyLoad(await api.loadPreset(name));
      } catch (e) {
        toast(String(e), "error");
      } finally {
        dismiss(loading);
      }
    },
    [toast, dismiss, applyLoad],
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

  useEffect(() => {
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
    });
    api.getCatalog().then(setCatalog);
    return on<Catalog>("catalog", setCatalog);
  }, []);

  /** Wire a freshly added node into the main chain, just before the Output. */
  const appendToChain = useCallback(async (id: number, view: StudioView) => {
    const out = view.nodes.find((n) => n.kind === "output");
    const feed = out && view.edges.find((e) => e.to === out.id && e.from !== id);
    if (!out || !feed) return view;
    await api.disconnect(feed.from, out.id);
    await api.connect(feed.from, id);
    return api.connect(id, out.id);
  }, []);

  /** Place a new chain node between the last chain node and the Output, shifting Output right. */
  const chainPosition = useCallback((): Pos => {
    const v = studioRef.current;
    const out = v?.nodes.find((n) => n.kind === "output");
    const feed = out && v?.edges.find((e) => e.to === out.id);
    const prev = feed && v?.nodes.find((n) => n.id === feed.from);
    if (!out || !prev) return { x: 400, y: 240 };
    const x = prev.x + 300;
    if (out.x < x + 300) api.moveNode(out.id, x + 300, out.y).catch(() => {});
    return { x, y: prev.y };
  }, []);

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
        await guard(() => api.setBypass(id, bypass));
        setStudio((s) => s && { ...s, nodes: s.nodes.map((n) => (n.id === id ? { ...n, bypass } : n)) });
      },
      openEditor: async (id) => {
        await nextFrame();
        await guard(() => api.openEditor(id));
      },
      loadPreset: loadPresetNow,
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
        next.bpm = Math.round(Math.max(30, Math.min(300, next.bpm)));
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
        const next = { ...cur.metronome, bpm: Math.round(Math.max(30, Math.min(300, bpm))) };
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
    [toast, guard, dismiss, appendToChain, chainPosition, loadPresetNow, applyLoad, applyView],
  );

  if (!studio) return IS_REMOTE ? <RemoteBoot /> : <div className="boot">Starting audio engine…</div>;

  if (IS_REMOTE)
    return (
      <StudioContext.Provider value={{ studio, catalog, presets, layoutKey, view: "board", stage: false, remote: true, actions }}>
        <RemoteShell />
        <Toasts toasts={toasts} onDismiss={dismiss} />
      </StudioContext.Provider>
    );

  return (
    <StudioContext.Provider value={{ studio, catalog, presets, layoutKey, view, stage, remote: false, actions }}>
      <ReactFlowProvider>
        <div className={`app view-${view} ${stage ? "stage-mode" : ""}`}>
          <TitleBar />
          <aside className="sidebar left">
            <PluginBrowser />
          </aside>
          <main className="stage">{view === "rig" ? <GraphCanvas /> : <PedalboardView />}</main>
          <Dock />
        </div>
        {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
        <Toasts toasts={toasts} onDismiss={dismiss} />
      </ReactFlowProvider>
    </StudioContext.Provider>
  );
}
