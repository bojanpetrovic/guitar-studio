// Takes the user manual's screenshots (docs/manual/images) from the real UI.
//
// It starts the docs server (src-tauri/examples/docs_server.rs: the app layer with a
// synthetic guitar and no audio device), serves the built UI (dist/), and drives headless
// Chrome over the DevTools protocol with scripts/docs/shim.js standing in for Tauri.
// No dependencies beyond Node 22+ and Google Chrome.
//
//   node scripts/docs/screenshots.mjs --data <scratch data dir> [--out docs/manual/images]
//   node scripts/docs/screenshots.mjs --data <dir> --serve   # keep running; POST code to :9400
//
// `make docs-screenshots` prepares the data dir and runs the tour (scripts/docs/tour.mjs).

import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { mkdirSync, readFileSync, writeFileSync, existsSync, mkdtempSync } from "node:fs";
import { join, extname, resolve } from "node:path";
import { tmpdir } from "node:os";

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const DATA = arg("data");
const OUT = resolve(arg("out", "docs/manual/images"));
const SERVE = process.argv.includes("--serve");
const WS_PORT = 7490;
const WEB_PORT = 7491;
const CDP_PORT = 9333;
const CONTROL_PORT = 9400;
const CHROME = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

if (!DATA) throw new Error("--data <dir> is required (a scratch copy of the app's data folder)");
if (!existsSync("dist/index.html")) throw new Error("build the UI first (npm run build)");
mkdirSync(OUT, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const children = [];
const quit = (code = 0) => {
  for (const c of children) c.kill();
  process.exit(code);
};
process.on("SIGINT", () => quit(130));
process.on("SIGTERM", () => quit(143));

// ---------------------------------------------------------------- servers

const server = spawn("target/release/examples/docs_server", [DATA, String(WS_PORT)], { stdio: ["ignore", "inherit", "inherit"] });
children.push(server);

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2", ".woff": "font/woff", ".json": "application/json" };
createServer((req, res) => {
  const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  let file = join("dist", path === "/" ? "index.html" : path);
  if (!existsSync(file) || file.includes("..")) file = "dist/index.html";
  res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
  res.end(readFileSync(file));
}).listen(WEB_PORT, "127.0.0.1");

// ---------------------------------------------------------------- chrome

const profile = mkdtempSync(join(tmpdir(), "gs-docs-chrome-"));
const chrome = spawn(CHROME, [
  "--headless=new",
  `--remote-debugging-port=${CDP_PORT}`,
  `--user-data-dir=${profile}`,
  "--hide-scrollbars",
  "--no-first-run",
  "--no-default-browser-check",
  "--window-size=1440,900",
  "about:blank",
], { stdio: "ignore" });
children.push(chrome);

async function cdpTarget() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      /* not up yet */
    }
    await sleep(200);
  }
  throw new Error("Chrome didn't start");
}

class Cdp {
  constructor(url) {
    this.ws = new WebSocket(url);
    this.next = 1;
    this.pending = new Map();
    this.ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
      } else if (msg.method === "Runtime.consoleAPICalled" && process.env.DOCS_CONSOLE) {
        console.log("[page]", ...msg.params.args.map((a) => a.value ?? a.description));
      } else if (msg.method === "Runtime.exceptionThrown") {
        console.log("[page error]", msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
      }
    };
    this.ready = new Promise((r) => (this.ws.onopen = r));
  }
  send(method, params = {}) {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
}

const cdp = new Cdp(await cdpTarget());
await cdp.ready;
await cdp.send("Page.enable");
await cdp.send("Runtime.enable");

// ---------------------------------------------------------------- helpers for tours

const shim = readFileSync("scripts/docs/shim.js", "utf8");
let shimScript = null;

const page = {
  /** Open the desktop UI (with the Tauri stand-in) or the phone page. */
  async open({ phone = false, width = 1440, height = 900, scale = 2, theme = "amber" } = {}) {
    if (shimScript) await cdp.send("Page.removeScriptToEvaluateOnNewDocument", { identifier: shimScript });
    shimScript = null;
    const prefs = `try { localStorage.setItem("gs.theme", ${JSON.stringify(theme)}); } catch {}`;
    const src = phone ? prefs : `${prefs}\n${shim}`;
    shimScript = (await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: src })).identifier;
    await cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: scale, mobile: phone });
    await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: phone });
    const url = phone
      ? `http://127.0.0.1:${WEB_PORT}/#t=docs&ws=127.0.0.1:${WS_PORT}`
      : `http://127.0.0.1:${WEB_PORT}/`;
    // Via a blank page: a new #hash alone wouldn't reload the app.
    await cdp.send("Page.navigate", { url: "about:blank" });
    await sleep(200);
    await cdp.send("Page.navigate", { url });
    await sleep(2500);
  },
  /** Run code in the page and return its (JSON) value. */
  async eval(fn, ...args) {
    const expression = `(${fn.toString()})(...${JSON.stringify(args)})`;
    const r = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  },
  /** Run an engine command directly (through the shim's socket). */
  call(cmd, args = {}) {
    return page.eval((c, a) => window.__gsDocs.call(c, a), cmd, args);
  },
  /** The centre of the first element matching `selector` (optionally containing `text`). */
  async center(selector, text) {
    const at = await page.eval(
      (sel, txt) => {
        const els = [...document.querySelectorAll(sel)].filter((e) => !txt || e.textContent.includes(txt));
        const el = els.find((e) => e.getBoundingClientRect().width > 0);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
      },
      selector,
      text ?? null,
    );
    if (!at) throw new Error(`nothing matches ${selector}${text ? ` "${text}"` : ""}`);
    return at;
  },
  async mouse(type, x, y, extra = {}) {
    await cdp.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1, ...extra });
  },
  /** A real click (pointer events and all). */
  async click(selector, text, { double = false } = {}) {
    const { x, y } = await page.center(selector, text);
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    for (let n = 1; n <= (double ? 2 : 1); n++) {
      await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: n });
      await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: n });
    }
    await sleep(300);
  },
  async hover(selector, text) {
    const { x, y } = await page.center(selector, text);
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await sleep(250);
  },
  /** Drag from one point to another in steps. */
  async drag(from, to, steps = 12) {
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...from });
    await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", ...from, button: "left", buttons: 1, clickCount: 1 });
    for (let i = 1; i <= steps; i++) {
      const x = from.x + ((to.x - from.x) * i) / steps;
      const y = from.y + ((to.y - from.y) * i) / steps;
      await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "left", buttons: 1 });
      await sleep(16);
    }
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...to, button: "left", buttons: 0, clickCount: 1 });
    await sleep(300);
  },
  /** A key, optionally with "Meta" / "Shift" / "Alt" / "Control" (e.g. key("2", ["Meta"])). */
  async key(key, mods = [], code) {
    const bits = { Alt: 1, Control: 2, Meta: 4, Shift: 8 };
    const modifiers = mods.reduce((m, k) => m | bits[k], 0);
    const c = code ?? (key.length === 1 ? (/\d/.test(key) ? `Digit${key}` : `Key${key.toUpperCase()}`) : key);
    const text = key.length === 1 && !(modifiers & 6) ? key : undefined;
    await cdp.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", key, code: c, modifiers, text, windowsVirtualKeyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0 });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code: c, modifiers });
    await sleep(250);
  },
  async type(text) {
    await cdp.send("Input.insertText", { text });
    await sleep(150);
  },
  /** Save a screenshot: the whole page, a selector's box (with `pad`), or a rect. */
  async shot(name, { selector, text, pad = 0, rect } = {}) {
    let clip = rect;
    if (selector) {
      clip = await page.eval(
        (sel, txt, p) => {
          const el = [...document.querySelectorAll(sel)].find((e) => (!txt || e.textContent.includes(txt)) && e.getBoundingClientRect().width > 0);
          if (!el) return null;
          const r = el.getBoundingClientRect();
          return { x: Math.max(0, r.x - p), y: Math.max(0, r.y - p), width: r.width + 2 * p, height: r.height + 2 * p };
        },
        selector,
        text ?? null,
        pad,
      );
      if (!clip) throw new Error(`nothing to shoot for ${selector}`);
    }
    const params = { format: "png", captureBeyondViewport: false };
    if (clip) params.clip = { ...clip, scale: 1 };
    const { data } = await cdp.send("Page.captureScreenshot", params);
    const file = join(OUT, `${name}.png`);
    writeFileSync(file, Buffer.from(data, "base64"));
    // Much smaller files at no visible cost, if pngquant is installed (brew install pngquant).
    spawnSync("pngquant", ["--quality", "70-90", "--speed", "1", "--force", "--ext", ".png", file]);
    console.log("shot", file);
    return file;
  },
  sleep,
};

// ---------------------------------------------------------------- run

await sleep(1500); // the docs server loading its data

if (SERVE) {
  // POST a function body; it runs with `page` in scope. Its return value comes back as JSON.
  const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
  createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    try {
      const out = await new AsyncFunction("page", body)(page);
      res.end(JSON.stringify(out ?? null, null, 1) + "\n");
    } catch (e) {
      res.statusCode = 500;
      res.end(String(e?.stack ?? e) + "\n");
    }
  }).listen(CONTROL_PORT, "127.0.0.1");
  console.log(`docs session ready: POST code to http://127.0.0.1:${CONTROL_PORT}`);
} else {
  const { default: tour } = await import(resolve("scripts/docs/tour.mjs"));
  try {
    await tour(page);
    quit(0);
  } catch (e) {
    console.error(e);
    quit(1);
  }
}
