import { useMemo, useState } from "react";
import { AudioWaveform, Loader2, Piano, RefreshCw, Search, SlidersHorizontal } from "lucide-react";
import { useStudio } from "../App";
import { formatLabel, PluginDescriptor, pluginKey } from "../api";

type FormatFilter = "all" | "vst3" | "au" | "clap";
type KindFilter = "effects" | "instruments" | "all";

export const DRAG_MIME = "application/x-gs-plugin";
export const DRAG_GAIN = "gs:builtin:gain";

function timeAgo(secs: number | null) {
  if (!secs) return "never";
  const d = Date.now() / 1000 - secs;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

export default function PluginBrowser() {
  const { catalog, actions } = useStudio();
  const [query, setQuery] = useState("");
  const [format, setFormat] = useState<FormatFilter>("all");
  const [kind, setKind] = useState<KindFilter>("effects");

  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    return catalog.plugins.filter((p) => {
      if (format !== "all" && !(format === "au" ? p.format.startsWith("au") : p.format === format)) return false;
      if (kind === "effects" && p.category === "instrument") return false;
      if (kind === "instruments" && p.category !== "instrument") return false;
      return !q || p.name.toLowerCase().includes(q) || p.vendor.toLowerCase().includes(q);
    });
  }, [catalog.plugins, query, format, kind]);

  const onDragStart = (e: React.DragEvent, key: string) => {
    e.dataTransfer.setData(DRAG_MIME, key);
    e.dataTransfer.effectAllowed = "copy";
  };

  return (
    <div className="browser">
      <div className="panel-head">
        <h2>Plugins</h2>
        <button
          className="icon-btn"
          onClick={actions.scan}
          disabled={catalog.scanning}
          title={`Rescan plugins (last scan: ${timeAgo(catalog.scannedAt)})`}
        >
          {catalog.scanning ? <Loader2 size={16} className="spin" /> : <RefreshCw size={16} />}
        </button>
      </div>

      <div className="search">
        <Search size={15} />
        <input placeholder="Search plugins or vendors" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>

      <div className="segmented small">
        {(["all", "vst3", "au", "clap"] as FormatFilter[]).map((f) => (
          <button key={f} className={format === f ? "on" : ""} onClick={() => setFormat(f)}>
            {f === "all" ? "All" : formatLabel[f]}
          </button>
        ))}
      </div>
      <div className="segmented small">
        {(["effects", "instruments", "all"] as KindFilter[]).map((k) => (
          <button key={k} className={kind === k ? "on" : ""} onClick={() => setKind(k)}>
            {k[0].toUpperCase() + k.slice(1)}
          </button>
        ))}
      </div>

      <div className="section-label">Built-in</div>
      <button
        className="plugin-item builtin"
        draggable
        onDragStart={(e) => onDragStart(e, DRAG_GAIN)}
        onClick={() => actions.addGain()}
      >
        <span className="plugin-icon">
          <SlidersHorizontal size={16} />
        </span>
        <span className="plugin-text">
          <span className="plugin-name">Gain</span>
          <span className="plugin-vendor">Level / volume utility</span>
        </span>
      </button>

      <div className="section-label">
        {catalog.plugins.length ? `${list.length} of ${catalog.plugins.length}` : "Installed"}
        <span className="muted"> · click to add to chain, drag to place</span>
      </div>

      <div className="plugin-list">
        {catalog.plugins.length === 0 ? (
          <div className="empty">
            <AudioWaveform size={28} />
            <p>{catalog.scanning ? "Scanning your plugins… this can take a minute or two the first time." : "No plugins scanned yet."}</p>
            {!catalog.scanning && (
              <button className="btn primary" onClick={actions.scan}>
                Scan plugins
              </button>
            )}
          </div>
        ) : (
          list.map((p: PluginDescriptor) => (
            <button
              key={pluginKey(p)}
              className="plugin-item"
              draggable
              onDragStart={(e) => onDragStart(e, pluginKey(p))}
              onClick={() => actions.addPlugin(p)}
              title={p.path}
            >
              <span className="plugin-icon">
                {p.category === "instrument" ? <Piano size={16} /> : <AudioWaveform size={16} />}
              </span>
              <span className="plugin-text">
                <span className="plugin-name">{p.name}</span>
                <span className="plugin-vendor">{p.vendor || "Unknown vendor"}</span>
              </span>
              <span className={`badge fmt-${p.format}`}>{formatLabel[p.format]}</span>
            </button>
          ))
        )}
      </div>

      {catalog.errors.length > 0 && (
        <details className="scan-errors">
          <summary>{catalog.errors.length} scan issue(s)</summary>
          {catalog.errors.map((e, i) => (
            <p key={i}>{e}</p>
          ))}
        </details>
      )}
    </div>
  );
}
