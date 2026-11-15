// Stands in for Tauri's IPC bridge (`window.__TAURI_INTERNALS__`) in a plain browser, so
// the desktop UI runs unchanged against the docs server (src-tauri/examples/docs_server.rs)
// for the manual's screenshots. Injected before the page's own scripts.
(() => {
  const url = window.__GS_DOCS_WS__ || "ws://127.0.0.1:7490/ws";
  const ws = new WebSocket(url);
  const open = new Promise((resolve) => (ws.onopen = resolve));
  const pending = new Map();
  const callbacks = new Map();
  const listeners = new Map();
  let nextId = 1;
  let nextCallback = 1;

  // The docs server runs host-driven (no device): show it as an audio interface would.
  const relabel = (v, depth = 0) => {
    if (!v || typeof v !== "object" || depth > 4) return v;
    if (v.inputDevice === "Plugin host") {
      Object.assign(v, { inputDevice: "Audio Interface", outputDevice: "Audio Interface", bufferSize: 64, latencyMs: 4 });
    }
    for (const k of Object.keys(v)) relabel(v[k], depth + 1);
    return v;
  };

  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.event) {
      relabel(msg.payload);
      for (const id of listeners.get(msg.event) ?? []) callbacks.get(id)?.({ event: msg.event, id, payload: msg.payload });
      return;
    }
    const p = pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    "err" in msg ? p.reject(msg.err) : p.resolve(relabel(msg.ok ?? null));
  };

  const request = async (cmd, args) => {
    await open;
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, cmd, args: args ?? {} }));
    });
  };

  // Window calls (stage mode): a 1440 × 900 window at 2×.
  const windowCalls = {
    scale_factor: 2,
    inner_size: { width: 2880, height: 1800 },
    outer_size: { width: 2880, height: 1800 },
    outer_position: { x: 0, y: 0 },
    inner_position: { x: 0, y: 0 },
    current_monitor: { name: "Display", size: { width: 3024, height: 1964 }, position: { x: 0, y: 0 }, scaleFactor: 2, workArea: { position: { x: 0, y: 0 }, size: { width: 3024, height: 1900 } } },
  };

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(callback) {
      const id = nextCallback++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback: (id) => callbacks.delete(id),
    convertFileSrc: (path) => path,
    async invoke(cmd, args) {
      if (cmd === "call" || cmd === "call_async") return request(args.cmd, args.args);
      if (cmd === "plugin:event|listen") {
        if (!listeners.has(args.event)) listeners.set(args.event, new Set());
        listeners.get(args.event).add(args.handler);
        return args.handler;
      }
      if (cmd === "plugin:event|unlisten") {
        listeners.get(args.event)?.delete(args.eventId);
        return null;
      }
      const name = cmd.replace(/^plugin:window\|/, "");
      if (name in windowCalls) return structuredClone(windowCalls[name]);
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event, id) => listeners.get(event)?.delete(id),
  };
  // For the driver script: run any command directly.
  window.__gsDocs = { call: request };
})();
