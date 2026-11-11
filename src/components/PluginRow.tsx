import { AudioWaveform, Piano } from "lucide-react";
import { formatLabel, PluginDescriptor } from "../api";

/** One plugin in a list: icon, name, vendor and format badge. */
export default function PluginRow({ p, className = "", ...rest }: { p: PluginDescriptor } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button className={`plugin-item ${className}`} title={p.path} {...rest}>
      <span className="plugin-icon">{p.category === "instrument" ? <Piano size={16} /> : <AudioWaveform size={16} />}</span>
      <span className="plugin-text">
        <span className="plugin-name">{p.name}</span>
        <span className="plugin-vendor">{p.vendor || "Unknown vendor"}</span>
      </span>
      <span className={`badge fmt-${p.format}`}>{formatLabel[p.format]}</span>
    </button>
  );
}
