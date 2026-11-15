import { useEffect, useRef, useState } from "react";
import { Check, Palette } from "lucide-react";
import { useDismiss } from "../hooks";
import { cycleTheme, setThemeChoice, Theme, THEMES, useTheme } from "../themes";
import { keys } from "../platform";

function Preview({ s }: { s: Theme["swatch"] }) {
  return (
    <svg className="theme-preview" viewBox="0 0 120 64" aria-hidden>
      <rect width="120" height="64" rx="7" fill={s.bg} />
      <rect x="0" y="0" width="120" height="10" fill={s.panel} />
      <circle cx="7" cy="5" r="1.6" fill={s.line} />
      <circle cx="12" cy="5" r="1.6" fill={s.line} />
      <rect x="6" y="16" width="34" height="24" rx="4" fill={s.panel} stroke={s.line} />
      <rect x="6" y="16" width="34" height="2" rx="1" fill={s.accent} />
      <rect x="10" y="23" width="20" height="2.4" rx="1.2" fill={s.text} opacity="0.75" />
      <rect x="10" y="30" width="26" height="2" rx="1" fill={s.line} />
      <rect x="10" y="30" width="15" height="2" rx="1" fill={s.accent} />
      <rect x="78" y="26" width="36" height="24" rx="4" fill={s.panel} stroke={s.line} />
      <rect x="78" y="26" width="36" height="2" rx="1" fill={s.accent2} />
      <rect x="82" y="33" width="22" height="2.4" rx="1.2" fill={s.text} opacity="0.75" />
      <path d="M40 28 C 58 28, 60 38, 78 38" fill="none" stroke={s.accent} strokeWidth="2" strokeLinecap="round" />
      <rect x="6" y="48" width="44" height="9" rx="4.5" fill={s.accent} />
    </svg>
  );
}

export default function ThemeMenu() {
  const { choice, theme } = useTheme();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  // ⌘⇧T steps through themes from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        cycleTheme(1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useDismiss(root, open, () => setOpen(false));

  const system = choice === "system";

  return (
    <div className="theme-menu" ref={root}>
      <button
        className={`icon-btn ${open ? "active" : ""}`}
        onClick={() => setOpen((o) => !o)}
        aria-label="Theme"
        title={keys(`Theme: ${theme.name} (⇧⌘T to cycle)`)}
      >
        <Palette size={17} />
      </button>

      {open && (
        <div className="popover theme-pop">
          <div className="pop-head">
            <span className="micro-label">Theme</span>
            <span className="muted mono">{keys("⇧⌘T")}</span>
          </div>
          <div className="theme-grid">
            {THEMES.map((t) => {
              const on = t.id === theme.id;
              return (
                <button
                  key={t.id}
                  className={`theme-card ${on ? "on" : ""}`}
                  onClick={() => setThemeChoice(t.id)}
                  title={t.blurb}
                >
                  <Preview s={t.swatch} />
                  <span className="theme-card-row">
                    <span className="theme-name">{t.name}</span>
                    {on && <Check size={13} className="theme-check" />}
                  </span>
                  <span className="theme-blurb">{t.blurb}</span>
                </button>
              );
            })}
          </div>
          <label className="theme-system">
            <span>
              Match system appearance
              <small>Amber when the system is dark, Daylight when light</small>
            </span>
            <button
              className={`switch ${system ? "on" : ""}`}
              onClick={() => setThemeChoice(system ? theme.id : "system")}
              role="switch"
              aria-checked={system}
            >
              <span />
            </button>
          </label>
        </div>
      )}
    </div>
  );
}
