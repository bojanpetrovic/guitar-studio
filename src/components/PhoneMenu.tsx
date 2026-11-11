// Title-bar popover that turns on the phone remote and shows its QR code.
import { useEffect, useRef, useState } from "react";
import { Check, Copy, RefreshCw, Smartphone } from "lucide-react";
import { api, RemoteStatus } from "../api";
import { on } from "../transport";
import { useStudio } from "../App";

export default function PhoneMenu() {
  const { actions } = useStudio();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [copied, setCopied] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.remoteStatus().then(setStatus, () => {});
    return on<RemoteStatus>("remote", setStatus);
  }, []);
  // The computer's address can change (other Wi-Fi): read it fresh on every open.
  useEffect(() => {
    if (open) api.remoteStatus().then(setStatus, () => {});
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => root.current && !root.current.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const enabled = !!status?.enabled;
  const clients = status?.clients ?? 0;
  const toggle = () =>
    api.setRemote(!enabled).then(setStatus, (e) => actions.toast(String(e), "error"));
  const renew = () =>
    api.renewRemoteLink().then(
      (s) => {
        setStatus(s);
        actions.toast("New link made — phones need to scan again", "info");
      },
      (e) => actions.toast(String(e), "error"),
    );
  const copy = () => {
    if (!status?.url) return;
    navigator.clipboard?.writeText(status.url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }, () => {});
  };

  return (
    <div className="phone-menu" ref={root}>
      <button
        className={`icon-btn ${open ? "active" : ""} ${clients > 0 ? "live" : ""}`}
        onClick={() => setOpen((o) => !o)}
        aria-label="Phone remote"
        title={clients > 0 ? `${clients} phone${clients === 1 ? "" : "s"} connected` : "Control the pedalboard from your phone"}
      >
        <Smartphone size={17} />
        {clients > 0 && <span className="phone-badge mono">{clients}</span>}
      </button>

      {open && (
        <div className="popover phone-pop">
          <div className="pop-head">
            <span className="micro-label">Phone remote</span>
            {status?.running && (
              <span className={`phone-count ${clients > 0 ? "on" : ""}`}>
                {clients > 0 ? `${clients} connected` : "Waiting for a phone"}
              </span>
            )}
          </div>

          <label className="phone-row">
            <span>
              Allow phones on this Wi-Fi
              <small>Play this rig’s pedalboard from your phone’s browser</small>
            </span>
            <button className={`switch ${enabled ? "on" : ""}`} onClick={toggle} role="switch" aria-checked={enabled}>
              <span />
            </button>
          </label>

          {status?.error && <div className="phone-error">{status.error}</div>}

          {status?.running && status.url && (
            <>
              <div className="phone-qr" dangerouslySetInnerHTML={{ __html: status.qrSvg ?? "" }} />
              <p className="phone-hint">Scan with your phone’s camera. Phone and computer must be on the same network.</p>
              <div className="phone-url">
                <span className="mono truncate" title={status.url}>
                  {status.url.replace(/#.*/, "")}
                </span>
                <button className="icon-btn ghost mini" onClick={copy} title="Copy the full link">
                  {copied ? <Check size={13} /> : <Copy size={13} />}
                </button>
              </div>
              <div className="phone-foot">
                <button className="btn small" onClick={renew} title="Phones using the current link are disconnected">
                  <RefreshCw size={12} /> New link
                </button>
                <span className="muted small">The link holds a secret key — share it only with your own devices.</span>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
