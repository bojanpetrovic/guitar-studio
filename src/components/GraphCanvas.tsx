import { useCallback, useEffect } from "react";
import {
  Background,
  BackgroundVariant,
  Connection,
  Controls,
  Edge,
  MiniMap,
  Node,
  ReactFlow,
  useEdgesState,
  useNodesState,
  useReactFlow,
} from "@xyflow/react";
import { AlertTriangle } from "lucide-react";
import { useStudio } from "../App";
import { api, NodeView } from "../api";
import { useTheme } from "../themes";
import StudioNode from "./StudioNode";
import SceneBar from "./SceneBar";
import SignalEdge from "./SignalEdge";
import { DRAG_GAIN, DRAG_MIME, DRAG_MIXER } from "./PluginBrowser";

export type StudioFlowNode = Node<{ node: NodeView }, "studio">;

const nodeTypes = { studio: StudioNode };
const edgeTypes = { signal: SignalEdge };

export default function GraphCanvas() {
  const { studio, catalog, layoutKey, actions } = useStudio();
  const { theme } = useTheme();
  const { screenToFlowPosition, fitView } = useReactFlow();
  const [nodes, setNodes, onNodesChange] = useNodesState<StudioFlowNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  // Engine state is the source of truth; keep local selection when it changes.
  // Reuse each previous flow node (or at least its measured size): a node object
  // without `measured` is hidden and re-measured by React Flow, and a fader drag
  // updates the engine view ~60×/s — fresh objects every time left the whole canvas
  // blank (all nodes stuck unmeasured).
  useEffect(() => {
    setNodes((prev) => {
      const byId = new Map(prev.map((p) => [p.id, p]));
      return studio.nodes.map((n) => {
        const id = String(n.id);
        const old = byId.get(id);
        if (old && old.data.node === n) return old;
        const position = old && old.position.x === n.x && old.position.y === n.y ? old.position : { x: n.x, y: n.y };
        return {
          ...old,
          id,
          type: "studio",
          position,
          data: { node: n },
          selected: old?.selected,
          deletable: n.kind !== "input" && n.kind !== "output",
        };
      });
    });
  }, [studio.nodes, setNodes]);

  useEffect(() => {
    setEdges((prev) =>
      studio.edges.map((e) => {
        const id = `${e.from}->${e.to}`;
        return {
          id,
          source: String(e.from),
          target: String(e.to),
          type: "signal",
          selected: prev.find((p) => p.id === id)?.selected,
        };
      }),
    );
  }, [studio.edges, setEdges]);

  useEffect(() => {
    if (!layoutKey) return;
    const t = setTimeout(() => fitView({ padding: 0.3, maxZoom: 1, duration: 300 }), 60);
    return () => clearTimeout(t);
  }, [layoutKey, fitView]);

  const onConnect = useCallback((c: Connection) => actions.connect(Number(c.source), Number(c.target)), [actions]);

  const isValidConnection = useCallback(
    (c: Connection | Edge) => c.source !== c.target,
    [],
  );

  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const key = e.dataTransfer.getData(DRAG_MIME);
      if (!key) return;
      const pos = screenToFlowPosition({ x: e.clientX - 120, y: e.clientY - 40 });
      if (key === DRAG_GAIN) {
        actions.addGain(pos);
        return;
      }
      if (key === DRAG_MIXER) {
        actions.addMixer(pos);
        return;
      }
      const plugin = catalog.plugins.find((p) => `${p.format}:${p.uniqueId}` === key);
      if (plugin) actions.addPlugin(plugin, pos);
    },
    [screenToFlowPosition, catalog.plugins, actions],
  );

  return (
    <div className="canvas" onDragOver={(e) => e.preventDefault()} onDrop={onDrop}>
      {!studio.audio && (
        <div className="banner">
          <AlertTriangle size={16} />
          <span>Audio is not running{studio.audioError ? `: ${studio.audioError}` : "."}</span>
          <button className="btn small" onClick={actions.openSettings}>
            Audio settings
          </button>
        </div>
      )}
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        isValidConnection={isValidConnection}
        onNodesDelete={(ns) => ns.forEach((n) => actions.remove(Number(n.id)))}
        onEdgesDelete={(es) => es.forEach((e) => actions.disconnect(Number(e.source), Number(e.target)))}
        onNodeDragStop={(_, __, dragged) =>
          dragged.forEach((n) => api.moveNode(Number(n.id), n.position.x, n.position.y))
        }
        deleteKeyCode={["Backspace", "Delete"]}
        fitView
        fitViewOptions={{ padding: 0.3, maxZoom: 1 }}
        minZoom={0.3}
        maxZoom={1.6}
        colorMode={theme.mode}
        // Space and ↑ / ↓ belong to the metronome (see Dock).
        panActivationKeyCode={null}
        disableKeyboardA11y
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} color="var(--grid-dot)" />
        <Controls showInteractive={false} position="bottom-left" />
        <MiniMap
          pannable
          zoomable
          position="bottom-right"
          nodeColor={(n) => {
            const kind = (n.data as { node: NodeView }).node.kind;
            return kind === "plugin" ? "var(--accent)" : kind === "gain" || kind === "mixer" ? "var(--blue)" : "var(--green)";
          }}
          maskColor="var(--minimap-mask)"
        />
      </ReactFlow>
      <SceneBar variant="canvas" />
    </div>
  );
}
