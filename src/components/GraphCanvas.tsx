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
import StudioNode from "./StudioNode";
import SignalEdge from "./SignalEdge";
import { DRAG_GAIN, DRAG_MIME, DRAG_MIXER } from "./PluginBrowser";

export type StudioFlowNode = Node<{ node: NodeView }, "studio">;

const nodeTypes = { studio: StudioNode };
const edgeTypes = { signal: SignalEdge };

export default function GraphCanvas() {
  const { studio, catalog, layoutKey, actions } = useStudio();
  const { screenToFlowPosition, fitView } = useReactFlow();
  const [nodes, setNodes, onNodesChange] = useNodesState<StudioFlowNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  // Engine state is the source of truth; keep local selection when it changes.
  useEffect(() => {
    setNodes((prev) =>
      studio.nodes.map((n) => {
        const id = String(n.id);
        const old = prev.find((p) => p.id === id);
        return {
          id,
          type: "studio",
          position: { x: n.x, y: n.y },
          data: { node: n },
          selected: old?.selected,
          deletable: n.kind !== "input" && n.kind !== "output",
        };
      }),
    );
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
        colorMode="dark"
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} color="rgba(255,255,255,0.08)" />
        <Controls showInteractive={false} position="bottom-left" />
        <MiniMap
          pannable
          zoomable
          position="bottom-right"
          nodeColor={(n) => {
            const kind = (n.data as { node: NodeView }).node.kind;
            return kind === "plugin" ? "#ff9f2e" : kind === "gain" || kind === "mixer" ? "#7aa2ff" : "#3ddc97";
          }}
          maskColor="rgba(8,9,12,0.7)"
        />
      </ReactFlow>
    </div>
  );
}
