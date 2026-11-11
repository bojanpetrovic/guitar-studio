import { memo, useMemo, useState } from "react";
import { AudioWaveform, Blend, Loader2, RefreshCw, Search, SlidersHorizontal } from "lucide-react";
import { useActions, useCatalog } from "../studio";
import { formatLabel, pluginKey, pluginMatches } from "../api";
import { DRAG_GAIN, DRAG_MIME, DRAG_MIXER, DRAG_REPLACEABLE } from "../dnd";
import { timeAgo } from "../util";
import PluginRow from "./PluginRow";

type FormatFilter = "all" | "vst3" | "au" | "clap";
type KindFilter = "effects" | "instruments" | "all";

/** The plugin list. Reads only the catalog and the actions, so a fader move elsewhere
 *  doesn't re-render hundreds of rows. */
export default memo(function PluginBrowser() {
  const catalog = useCatalog();
  const actions = useActions();
  const [query, setQuery] = useState("");
  const [format, setFormat] = useState<FormatFilter>("all");
  const [kind, setKind] = useState<KindFilter>("effects");

  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    return catalog.plugins.filter((p) => {
      if (format !== "all" && !(format === "au" ? p.format.startsWith("au") : p.format === format)) return false;
      if (kind === "effects" && p.category === "instrument") return false;
      if (kind === "instruments" && p.category !== "instrument") return false;
      return pluginMatches(p, q);
    });
  }, [catalog.plugins, query, format, kind]);

  const onDragStart = (e: React.DragEvent, key: string) => {
    e.dataTransfer.setData(DRAG_MIME, key);
    if (key !== DRAG_GAIN && key !== DRAG_MIXER) e.dataTransfer.setData(DRAG_REPLACEABLE, "1");
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
      <button
        className="plugin-item builtin"
        draggable
        onDragStart={(e) => onDragStart(e, DRAG_MIXER)}
        onClick={() => actions.addMixer()}
      >
        <span className="plugin-icon">
          <Blend size={16} />
        </span>
        <span className="plugin-text">
          <span className="plugin-name">Mixer</span>
          <span className="plugin-vendor">Volume, pan & mute per incoming cable</span>
        </span>
      </button>

      <div className="section-label">
        {catalog.plugins.length ? `${list.length} of ${catalog.plugins.length}` : "Installed"}
        <span className="muted"> · click to add to chain, drag to place, drop on a plugin to replace it</span>
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
          list.map((p) => (
            <PluginRow
              key={pluginKey(p)}
              p={p}
              draggable
              onDragStart={(e) => onDragStart(e, pluginKey(p))}
              onClick={() => actions.addPlugin(p)}
            />
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
});
