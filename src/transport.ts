// How the UI reaches the engine:
// - the desktop app: Tauri's IPC (`call` / `call_async`, see src-tauri/src/lib.rs);
// - a phone (the same UI served over the local network, see src-tauri/src/remote.rs):
//   a WebSocket carrying the same commands and events;
// - Guitar Studio running as a plugin inside a DAW (crates/gs-plugin): the plugin
//   window's webview bridge, again with the same commands and events.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";

export const IS_DESKTOP = "__TAURI_INTERNALS__" in window;
/** Running as a plugin inside a DAW (the plugin's webview sets this flag). */
export const IS_PLUGIN = "__GS_PLUGIN__" in window;
/** Running in a phone's browser, remote-controlling the desktop app. */
export const IS_REMOTE = !IS_DESKTOP && !IS_PLUGIN;

/** Commands that only read; anything else counts as an edit (see `lastEdit`). */
const READS = /^(get_|list_|remote_)|^(plugin_params|preset_ready|open_editor)$/;
/** Commands the engine runs off its main thread (`gs_app::runs_off_main`). */
const OFF_MAIN = /^(scan_plugins|preset_ready)$/;

let editedAt = -Infinity;
/** When this window last changed something in the engine (performance.now()). */
export const lastEdit = () => editedAt;

export function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!READS.test(cmd)) editedAt = performance.now();
  if (IS_DESKTOP) return invoke<T>(OFF_MAIN.test(cmd) ? "call_async" : "call", { cmd, args });
  return link.call<T>(cmd, args);
}

/** Subscribe to an engine event. Returns the unsubscribe function. */
export function on<T>(event: string, fn: (payload: T) => void): () => void {
  if (IS_DESKTOP) {
    const un = listen<T>(event, (e) => fn(e.payload));
    return () => void un.then((f) => f());
  }
  return link.on(event, fn as (p: unknown) => void);
}

// ---------------------------------------------------------------- request / reply

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}

/** Generous: loading a preset with a big amp sim takes a few seconds. */
const CALL_TIMEOUT_MS = 20_000;

/** Numbered requests, their replies, and pushed events: what the phone's socket and the
 *  plugin's bridge have in common. */
abstract class Rpc {
  private nextId = 1;
  protected pending = new Map<number, Pending>();
  private handlers = new Map<string, Set<(p: unknown) => void>>();

  /** Hand a request to the other side. */
  protected abstract send(msg: { id: number; cmd: string; args: Record<string, unknown> }): void;
  /** Resolves once requests can be sent. */
  protected async ready() {}

  async call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
    await this.ready();
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      // A reply that never comes (the app hung, a dropped frame) mustn't wait forever.
      const t = window.setTimeout(() => {
        this.pending.delete(id);
        reject("Guitar Studio didn't answer");
      }, CALL_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: (v) => (window.clearTimeout(t), resolve(v as T)),
        reject: (e) => (window.clearTimeout(t), reject(e)),
      });
      this.send({ id, cmd, args });
    });
  }

  /** A reply arrived: `{id, ok}` or `{id, err}`. */
  protected settle(msg: { id: number; ok?: unknown; err?: unknown }) {
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if ("err" in msg) p.reject(msg.err);
    else p.resolve(msg.ok);
  }

  on(event: string, fn: (p: unknown) => void) {
    if (!this.handlers.has(event)) this.handlers.set(event, new Set());
    this.handlers.get(event)!.add(fn);
    return () => void this.handlers.get(event)!.delete(fn);
  }

  protected emit(event: string, payload: unknown) {
    this.handlers.get(event)?.forEach((fn) => fn(payload));
  }
}

// ---------------------------------------------------------------- plugin bridge

declare global {
  interface Window {
    ipc?: { postMessage(message: string): void };
    /** Called by the plugin (crates/gs-plugin/src/editor.rs) with replies and events. */
    __gs?: {
      reply(msg: { id: number; ok?: unknown; err?: unknown }): void;
      event(name: string, payload: unknown): void;
    };
  }
}

/** Inside a DAW: requests go out through the webview's message handler; the plugin
 *  answers by calling `window.__gs`. */
class PluginBridge extends Rpc {
  link: Link = "open";

  start() {
    window.__gs = {
      reply: (msg) => this.settle(msg),
      event: (name, payload) => this.emit(name, payload),
    };
  }

  protected send(msg: { id: number; cmd: string; args: Record<string, unknown> }) {
    window.ipc!.postMessage(JSON.stringify(msg));
  }

  subscribe = () => () => {};
}

// ---------------------------------------------------------------- phone connection

export type Link = "connecting" | "open" | "lost" | "denied" | "nolink";

const TOKEN_KEY = "gs.remote.token";
const HOST_KEY = "gs.remote.ws";

/** The QR link carries `#t=<token>` (and `&ws=<host:port>` from a dev build); keep them for reloads. */
function linkParams() {
  const hash = new URLSearchParams(location.hash.slice(1));
  const store = (k: string, v: string | null) => {
    try {
      if (v) localStorage.setItem(k, v);
      return v ?? localStorage.getItem(k);
    } catch {
      return v;
    }
  };
  return {
    token: store(TOKEN_KEY, hash.get("t")),
    host: store(HOST_KEY, hash.get("ws")) ?? location.host,
  };
}

class Socket extends Rpc {
  link: Link = "connecting";
  private ws: WebSocket | null = null;
  private waiting: (() => void)[] = [];
  private linkListeners = new Set<() => void>();
  private retry = 0;
  private timer: number | undefined;

  start() {
    // Scanning a new link into an open page only changes the #hash: start over with it.
    window.addEventListener(
      "hashchange",
      () => new URLSearchParams(location.hash.slice(1)).get("t") && location.reload(),
    );
    const { token } = linkParams();
    if (!token) return this.setLink("nolink");
    this.connect();
    // Phones drop sockets while asleep: reconnect as soon as the page is back.
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible" && this.link === "lost") this.connect();
    });
  }

  private connect() {
    window.clearTimeout(this.timer);
    const { token, host } = linkParams();
    const ws = new WebSocket(`ws://${host}/ws?t=${encodeURIComponent(token ?? "")}`);
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 0;
      this.setLink("open");
      this.waiting.splice(0).forEach((go) => go());
      this.emit("reconnected", null);
    };
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data as string);
      if (msg.event) {
        if (msg.event === "denied") return this.setLink("denied");
        return this.emit(msg.event, msg.payload);
      }
      this.settle(msg);
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.pending.forEach((p) => p.reject("Lost the connection to Guitar Studio"));
      this.pending.clear();
      if (this.link === "denied") return;
      this.setLink("lost");
      const delay = Math.min(5000, 400 * 2 ** this.retry++);
      this.timer = window.setTimeout(() => this.connect(), delay);
    };
  }

  protected async ready() {
    if (this.ws?.readyState === WebSocket.OPEN) return;
    if (this.link === "denied" || this.link === "nolink") throw "Not connected to Guitar Studio";
    await new Promise<void>((resolve, reject) => {
      const go = () => (window.clearTimeout(t), resolve());
      const t = window.setTimeout(() => {
        this.waiting = this.waiting.filter((w) => w !== go);
        reject("Can't reach Guitar Studio");
      }, 8000);
      this.waiting.push(go);
    });
  }

  protected send(msg: { id: number; cmd: string; args: Record<string, unknown> }) {
    this.ws!.send(JSON.stringify(msg));
  }

  private setLink(link: Link) {
    this.link = link;
    this.linkListeners.forEach((l) => l());
  }

  subscribe = (l: () => void) => {
    this.linkListeners.add(l);
    return () => void this.linkListeners.delete(l);
  };
}

const link = IS_PLUGIN ? new PluginBridge() : new Socket();
if (!IS_DESKTOP) link.start();

/** The phone's connection state (always "open" on the desktop and in a plugin). */
export function useLink(): Link {
  return useSyncExternalStore(link.subscribe, () => (IS_DESKTOP ? "open" : link.link));
}
