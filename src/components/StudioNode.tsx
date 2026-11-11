import { memo } from "react";
import { Handle, NodeProps, Position } from "@xyflow/react";
import { AudioWaveform, Blend, Cable, ExternalLink, Power, Replace, Speaker, SlidersHorizontal, Trash2 } from "lucide-react";
import { useStudio } from "../App";
import { formatLabel } from "../api";
import type { StudioFlowNode } from "./GraphCanvas";
import Fader, { formatDb } from "./Fader";
import Meter from "./Meter";
import MixerStrips from "./MixerStrips";

function StudioNode({ data, selected }: NodeProps<StudioFlowNode>) {
  const n = data.node;
  const { studio, actions } = useStudio();
  const inputChannels = studio.audio?.inputChannels ?? 2;

  const header = {
    input: { icon: <Cable size={16} />, title: "Guitar In", sub: studio.audio?.inputDevice ?? "No input" },
    output: { icon: <Speaker size={16} />, title: "Main Out", sub: studio.audio?.outputDevice ?? "No output" },
    gain: { icon: <SlidersHorizontal size={16} />, title: "Gain", sub: "Utility" },
    mixer: {
      icon: <Blend size={16} />,
      title: "Mixer",
      sub: `${n.strips.length} input${n.strips.length === 1 ? "" : "s"}`,
    },
    plugin: {
      icon: <AudioWaveform size={16} />,
      title: n.plugin?.name ?? "Plugin",
      sub: n.plugin?.vendor || "Plugin",
    },
  }[n.kind];

  return (
    <div
      className={`gs-node kind-${n.kind} ${n.bypass ? "bypassed" : ""} ${selected ? "selected" : ""}`}
      style={n.kind === "mixer" ? { width: Math.max(240, 28 + n.strips.length * 74) } : undefined}
    >
      {n.kind !== "input" && <Handle type="target" position={Position.Left} className="port in" />}

      <div className="node-head">
        <span className="node-icon">{header.icon}</span>
        <span className="node-titles">
          <span className="node-title">{header.title}</span>
          <span className="node-sub">{header.sub}</span>
        </span>
        {n.plugin && <span className={`badge fmt-${n.plugin.format}`}>{formatLabel[n.plugin.format]}</span>}
      </div>

      <div className="node-body">
        {n.kind === "input" && (
          <>
            <label className="select-row nodrag">
              <span>Channel</span>
              <select value={n.channel} onChange={(e) => actions.setParam(n.id, "channel", Number(e.target.value))}>
                {Array.from({ length: inputChannels }, (_, i) => (
                  <option key={i} value={i}>
                    Input {i + 1}
                  </option>
                ))}
              </select>
            </label>
            <Fader label="Trim" unit="dB" min={-24} max={24} value={n.gainDb} format={formatDb} resetTo={0}
              onChange={(v) => actions.setParam(n.id, "gainDb", v)} />
          </>
        )}

        {n.kind === "mixer" && <MixerStrips mixer={n.id} strips={n.strips} />}

        {(n.kind === "gain" || n.kind === "output" || n.kind === "mixer") && (
          <Fader label={n.kind === "output" ? "Master" : n.kind === "mixer" ? "Mix level" : "Level"} unit="dB" min={-60} max={n.kind === "output" ? 12 : 24}
            value={n.gainDb} format={formatDb} resetTo={0} onChange={(v) => actions.setParam(n.id, "gainDb", v)} />
        )}

        {n.kind === "plugin" && n.plugin && (
          <div className="node-actions nodrag">
            <button className="btn small" onClick={() => actions.openEditor(n.id)} disabled={!n.plugin.hasEditor}>
              <ExternalLink size={13} /> Open
            </button>
            <span className="spacer" />
            <button className="icon-btn" onClick={() => actions.pickReplacement(n.id)} title="Replace with another plugin (or drop one from the list onto this node)">
              <Replace size={14} />
            </button>
            <button
              className={`icon-btn toggle ${n.bypass ? "" : "on"}`}
              onClick={() => actions.setBypass(n.id, !n.bypass)}
              title={n.bypass ? "Enable" : "Bypass"}
            >
              <Power size={14} />
            </button>
            <button className="icon-btn danger" onClick={() => actions.remove(n.id)} title="Remove">
              <Trash2 size={14} />
            </button>
          </div>
        )}

        {(n.kind === "gain" || n.kind === "mixer") && (
          <div className="node-actions nodrag">
            <span className="spacer" />
            <button className="icon-btn danger" onClick={() => actions.remove(n.id)} title="Remove">
              <Trash2 size={14} />
            </button>
          </div>
        )}
      </div>

      <div className="node-meter">
        <Meter id={n.id} />
      </div>

      {n.kind !== "output" && <Handle type="source" position={Position.Right} className="port out" />}
    </div>
  );
}

export default memo(StudioNode);
