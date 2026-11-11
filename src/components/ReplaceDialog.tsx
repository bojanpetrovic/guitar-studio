// "Replace with…": pick another plugin for a plugin node. The node keeps its cables,
// place and on/off state; pedalboard controls follow where parameter names match.
import { useEffect, useMemo, useRef, useState } from "react";
import { Replace, Search, X } from "lucide-react";
import { useActions, useCatalog } from "../studio";
import { NodeView, PluginDescriptor, pluginKey, pluginMatches } from "../api";
import { useFavorites } from "../favorites";
import PluginRow from "./PluginRow";

export default function ReplaceDialog({ node, onClose }: { node: NodeView; onClose: () => void }) {
  const catalog = useCatalog();
  const actions = useActions();
  const current = node.plugin!;
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const favorites = useFavorites();

  // Same kind as the current plugin (effect / instrument): favourites first, then its
  // vendor's plugins, then the rest.
  const { list, starred, sameVendor } = useMemo(() => {
    const q = query.trim().toLowerCase();
    const instrument = current.category === "instrument";
    const fav = new Set(favorites);
    const rank = (p: PluginDescriptor) => (fav.has(pluginKey(p)) ? 0 : current.vendor && p.vendor === current.vendor ? 1 : 2);
    const list = catalog.plugins
      .filter((p) => pluginKey(p) !== pluginKey(current))
      .filter((p) => (p.category === "instrument") === instrument)
      .filter((p) => pluginMatches(p, q))
      .sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
    const count = (r: number) => (q ? 0 : list.filter((p) => rank(p) === r).length);
    return { list, starred: count(0), sameVendor: count(1) };
  }, [catalog.plugins, query, current, favorites]);

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
    <PluginRow key={pluginKey(p)} p={p} className={i === at ? "at" : ""} onClick={() => pick(p)} onMouseMove={() => setAt(i)} />
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
          {starred > 0 && <div className="section-label">Favourites</div>}
          {list.slice(0, starred).map(item)}
          {sameVendor > 0 && <div className="section-label">More from {current.vendor}</div>}
          {list.slice(starred, starred + sameVendor).map((p, i) => item(p, i + starred))}
          {starred + sameVendor > 0 && list.length > starred + sameVendor && <div className="section-label">All plugins</div>}
          {list.slice(starred + sameVendor).map((p, i) => item(p, i + starred + sameVendor))}
        </div>
        <p className="hint">
          Cables, position and on/off stay. The new plugin starts at its own defaults, and scenes forget the old
          one's settings. Pedalboard controls move to parameters with the same name.
        </p>
      </div>
    </div>
  );
}
