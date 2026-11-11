// Favourite plugins: kept at the top of the plugin list and the "Replace with…"
// picker. Saved by the engine (favorites.json in the app's folder), so the desktop app
// and the plugin version share them.
import { useSyncExternalStore } from "react";
import { api } from "./api";

let keys: string[] = [];
let requested = false;
const listeners = new Set<() => void>();

function set(next: string[]) {
  keys = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  if (!requested) {
    requested = true;
    api.getFavorites().then(set, () => (requested = false));
  }
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** The favourites' plugin keys. */
export const useFavorites = () => useSyncExternalStore(subscribe, () => keys);

/** Star or unstar a plugin (shown at once; put back if saving fails). */
export async function toggleFavorite(key: string) {
  const before = keys;
  const on = !keys.includes(key);
  set(on ? [...keys, key] : keys.filter((k) => k !== key));
  try {
    set(await api.setFavorite(key, on));
  } catch (e) {
    set(before);
    throw e;
  }
}
