// The plugin sidebar's open / closed state (⌘B or the title bar button), remembered
// per window.
import { useSyncExternalStore } from "react";

const KEY = "gs.sidebar";

function stored() {
  try {
    return localStorage.getItem(KEY) !== "closed";
  } catch {
    return true;
  }
}

let open = stored();
const listeners = new Set<() => void>();

export function toggleSidebar() {
  open = !open;
  try {
    localStorage.setItem(KEY, open ? "open" : "closed");
  } catch {
    /* not remembered: fine */
  }
  listeners.forEach((l) => l());
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

export const useSidebarOpen = () => useSyncExternalStore(subscribe, () => open);
