// "Replace with…": pick another plugin for a plugin node. The node keeps its cables,
// place and on/off state; pedalboard controls follow where parameter names match.
import { useEffect, useMemo, useRef, useState } from "react";
import { AudioWaveform, Piano, Replace, Search, X } from "lucide-react";
import { useStudio } from "../App";
import { formatLabel, NodeView, PluginDescriptor, pluginKey } from "../api";

export default function ReplaceDialog({ node, onClose }: { node: NodeView; onClose: () => void }) {
  const { catalog, actions } = useStudio();
  const current = node.plugin!;
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  // Same kind as the current plugin (effect / instrument); its vendor's plugins first.
  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    const instrument = current.category === "instrument";
    return catalog.plugins
      .filter((p) => pluginKey(p) !== pluginKey(current))
      .filter((p) => (p.category === "instrument") === instrument)
      .filter((p) => !q || p.name.toLowerCase().includes(q) || p.vendor.toLowerCase().includes(q))
      .sort((a, b) => Number(b.vendor === current.vendor) - Number(a.vendor === current.vendor) || a.name.localeCompare(b.name));
  }, [catalog.plugins, query, current]);
  const sameVendor = query ? 0 : list.filter((p) => p.vendor === current.vendor && current.vendor).length;

  useEffect(() => setAt(0), [query]);
  useEffect(() => {
    listRef.current?.querySelector(".plugin-item.at")?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const pick = (p: PluginDescriptor) => {
    onClose();
    actions.replacePlugin(node.id, p);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") setAt((i) => Math.min(list.length - 1, i + 1));
    else if (e.key === "ArrowUp") setAt((i) => Math.max(0, i - 1));
    else if (e.key === "Enter" && list[at]) pick(list[at]);
    else if (e.key === "Escape") onClose();
    else return;
    e.preventDefault();
  };

  const item = (p: PluginDescriptor, i: number) => (
    <button
      key={pluginKey(p)}
      className={`plugin-item ${i === at ? "at" : ""}`}
      onClick={() => pick(p)}
      onMouseMove={() => setAt(i)}
      title={p.path}
    >
      <span className="plugin-icon">{p.category === "instrument" ? <Piano size={16} /> : <AudioWaveform size={16} />}</span>
      <span className="plugin-text">
        <span className="plugin-name">{p.name}</span>
        <span className="plugin-vendor">{p.vendor || "Unknown vendor"}</span>
      </span>
      <span className={`badge fmt-${p.format}`}>{formatLabel[p.format]}</span>
    </button>
  );

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal replace-modal" onMouseDown={(e) => e.stopPropagation()} onKeyDown={onKey}>
        <div className="panel-head">
          <h2>
            <Replace size={16} /> Replace {current.name}
          </h2>
          <button className="icon-btn ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="search">
          <Search size={15} />
          <input autoFocus placeholder="Search plugins or vendors" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <div className="replace-list" ref={listRef}>
          {list.length === 0 && <p className="muted replace-empty">No other {current.category === "instrument" ? "instruments" : "effects"} match.</p>}
          {sameVendor > 0 && <div className="section-label">More from {current.vendor}</div>}
          {list.slice(0, sameVendor).map(item)}
          {sameVendor > 0 && list.length > sameVendor && <div className="section-label">All plugins</div>}
          {list.slice(sameVendor).map((p, i) => item(p, i + sameVendor))}
        </div>
        <p className="hint">
          Cables, position and on/off stay. The new plugin starts at its own defaults, and scenes forget the old
          one's settings. Pedalboard controls move to parameters with the same name.
        </p>
      </div>
    </div>
  );
}
