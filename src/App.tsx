import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ReactFlowProvider } from "@xyflow/react";
import {
  api,
  Catalog,
  nextFrame,
  PluginDescriptor,
  pluginKey,
  PRESET_EXISTS,
  PresetList,
  PresetLoad,
  StudioView,
} from "./api";
import TitleBar from "./components/TitleBar";
import PluginBrowser from "./components/PluginBrowser";
import GraphCanvas from "./components/GraphCanvas";
import TunerPanel from "./components/TunerPanel";
import MetronomePanel from "./components/MetronomePanel";
import SettingsDialog from "./components/SettingsDialog";
import Toasts, { Toast } from "./components/Toasts";

type Pos = { x: number; y: number };

export interface StudioActions {
  addPlugin: (p: PluginDescriptor, pos?: Pos) => Promise<void>;
  addGain: (pos?: Pos) => Promise<void>;
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
}

interface Ctx {
  studio: StudioView;
  catalog: Catalog;
  presets: PresetList;
  /** Bumps whenever a whole rig is swapped in, so the canvas can re-fit. */
  layoutKey: number;
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
  const toastId = useRef(1);
  const studioRef = useRef<StudioView | null>(null);
  studioRef.current = studio;

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

  useEffect(() => {
    // On launch, bring back the rig that was active last time (only into a blank studio,
    // so a dev hot-reload doesn't clobber work in progress).
    Promise.all([api.getStudio(), api.listPresets()]).then(([s, p]) => {
      setStudio(s);
      setPresets(p);
      const blank = s.nodes.length <= 2;
      if (blank && !p.current && p.last && p.presets.some((x) => x.name === p.last)) loadPresetNow(p.last);
    });
    api.getCatalog().then(setCatalog);
    const un = listen<Catalog>("catalog", (e) => setCatalog(e.payload));
    return () => {
      un.then((f) => f());
    };
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
      apply: setStudio,
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
        setStudio(autoWire ? await appendToChain(id, view) : view);
      },
      addGain: async (pos) => {
        const autoWire = !pos;
        const at = pos ?? chainPosition();
        const res = await guard(() => api.addGainNode(at.x, at.y));
        if (!res) return;
        const [id, view] = res;
        setStudio(autoWire ? await appendToChain(id, view) : view);
      },
      remove: async (id) => {
        const v = await guard(() => api.removeNode(id));
        if (v) setStudio(v);
      },
      connect: async (from, to) => {
        const v = await guard(() => api.connect(from, to));
        if (v) setStudio(v);
      },
      disconnect: async (from, to) => {
        const v = await guard(() => api.disconnect(from, to));
        if (v) setStudio(v);
      },
      setParam: (id, param, value) => {
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
      scan: async () => {
        const c = await guard(() => api.scanPlugins());
        if (c) {
          setCatalog(c);
          toast(`Found ${c.plugins.length} plugins`, "success");
        }
      },
    }),
    [toast, guard, dismiss, appendToChain, chainPosition, loadPresetNow, applyLoad],
  );

  if (!studio) return <div className="boot">Starting audio engine…</div>;

  return (
    <StudioContext.Provider value={{ studio, catalog, presets, layoutKey, actions }}>
      <ReactFlowProvider>
        <div className="app">
          <TitleBar />
          <aside className="sidebar left">
            <PluginBrowser />
          </aside>
          <main className="stage">
            <GraphCanvas />
          </main>
          <aside className="sidebar right">
            <TunerPanel />
            <MetronomePanel />
          </aside>
        </div>
        {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)} />}
        <Toasts toasts={toasts} onDismiss={dismiss} />
      </ReactFlowProvider>
    </StudioContext.Provider>
  );
}
