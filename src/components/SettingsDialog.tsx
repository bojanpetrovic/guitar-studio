import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";
import { useStudio } from "../studio";
import { api, AudioConfig, DeviceList } from "../api";

const BUFFER_SIZES = [32, 64, 128, 256, 512, 1024];

/** ASIO drivers (Windows) are listed with this prefix (see `gs_engine::audio::ASIO_PREFIX`). */
const ASIO = "ASIO: ";
const isAsio = (name: string | null | undefined) => !!name?.startsWith(ASIO);

/**
 * An ASIO driver does input and output together, and only one can be open: picking one
 * on either side picks it on the other too (and leaving it on one side leaves both).
 */
function pickDevice(cfg: AudioConfig, side: "inputDevice" | "outputDevice", name: string | null): AudioConfig {
  const other = side === "inputDevice" ? "outputDevice" : "inputDevice";
  if (isAsio(name)) return { ...cfg, [side]: name, [other]: name };
  if (isAsio(cfg[other])) return { ...cfg, [side]: name, [other]: null };
  return { ...cfg, [side]: name };
}

export default function SettingsDialog({ onClose }: { onClose: () => void }) {
  const { studio, actions } = useStudio();
  const [devices, setDevices] = useState<DeviceList | null>(null);
  const [cfg, setCfg] = useState<AudioConfig>(studio.audioConfig);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.listDevices().then(setDevices, (e) => actions.toast(String(e), "error"));
  }, [actions]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const apply = async () => {
    setBusy(true);
    try {
      actions.apply(await api.setAudioConfig(cfg));
      actions.toast("Audio restarted", "success");
      onClose();
    } catch (e) {
      actions.toast(String(e), "error");
      actions.apply(await api.getStudio());
    } finally {
      setBusy(false);
    }
  };

  const a = studio.audio;
  const sr = a?.sampleRate ?? 48000;

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="panel-head">
          <h2>Audio settings</h2>
          <button className="icon-btn ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>

        <label className="field">
          <span>Input device</span>
          <select value={cfg.inputDevice ?? ""} onChange={(e) => setCfg(pickDevice(cfg, "inputDevice", e.target.value || null))}>
            <option value="">System default</option>
            {devices?.inputs.map((d) => (
              <option key={d.name} value={d.name}>
                {d.name} ({d.channels} ch)
              </option>
            ))}
          </select>
        </label>

        <label className="field">
          <span>Output device</span>
          <select value={cfg.outputDevice ?? ""} onChange={(e) => setCfg(pickDevice(cfg, "outputDevice", e.target.value || null))}>
            <option value="">System default</option>
            {devices?.outputs.map((d) => (
              <option key={d.name} value={d.name}>
                {d.name} ({d.channels} ch)
              </option>
            ))}
          </select>
        </label>

        <div className="field">
          <span>Buffer size</span>
          <div className="segmented">
            {BUFFER_SIZES.map((b) => (
              <button key={b} className={cfg.bufferSize === b ? "on" : ""} onClick={() => setCfg({ ...cfg, bufferSize: b })}>
                {b}
              </button>
            ))}
          </div>
          <small className="muted">
            ≈ {((3 * cfg.bufferSize) / sr * 1000).toFixed(1)} ms round trip at {(sr / 1000).toFixed(1)} kHz. Lower feels
            tighter but costs more CPU; 64–128 is the sweet spot for playing live.
          </small>
        </div>

        {isAsio(cfg.inputDevice) ? (
          <p className="hint">
            An ASIO driver runs input and output together. If the buffer size or sample rate doesn't follow these
            settings, change them in the driver's own control panel. ASIO is a trademark and software of Steinberg
            Media Technologies GmbH.
          </p>
        ) : (
          <p className="hint">
            Tip: use the same audio interface for input and output so both run on one clock. Mixing devices (e.g. interface
            in, laptop speakers out) works, but adds jitter.
            {devices?.inputs.some((d) => isAsio(d.name)) && " On Windows, an ASIO driver gives the lowest latency."}
          </p>
        )}

        <div className="modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" onClick={apply} disabled={busy}>
            {busy && <Loader2 size={14} className="spin" />} Apply
          </button>
        </div>
      </div>
    </div>
  );
}
