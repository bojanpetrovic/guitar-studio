// The two side panels — presets on the left, plugins on the right — and whether each
// is open (title bar buttons, ⌘P / ⌘B). Remembered per window.
import { useSyncExternalStore } from "react";

export type Side = "left" | "right";

const key = (side: Side) => `gs.sidebar.${side}`;

function stored(side: Side) {
  try {
    return localStorage.getItem(key(side)) !== "closed";
  } catch {
    return true;
  }
}

const open: Record<Side, boolean> = { left: stored("left"), right: stored("right") };
const listeners = new Set<() => void>();

export function setSidebar(side: Side, on: boolean) {
  if (open[side] === on) return;
  open[side] = on;
  try {
    localStorage.setItem(key(side), on ? "open" : "closed");
  } catch {
    /* not remembered: fine */
  }
  listeners.forEach((l) => l());
}

export const toggleSidebar = (side: Side) => setSidebar(side, !open[side]);

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

export const useSidebarOpen = (side: Side) => useSyncExternalStore(subscribe, () => open[side]);
