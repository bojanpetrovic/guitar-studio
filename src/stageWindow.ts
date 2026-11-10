// Stage mode: while the pedalboard is being played, the main window looks frameless
// (traffic lights hidden; the overlay title bar is already invisible), turns
// see-through and takes the board's shape, so only the board is on screen.
// Leaving stage mode puts the window back exactly where and how it was.
import { invoke } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { setWindowTransparent } from "./themes";

const IN_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
/** Must match tauri.conf.json. */
const NORMAL_MIN = new LogicalSize(1100, 680);
const STAGE_MIN = new LogicalSize(360, 180);

type Saved = { size: LogicalSize; pos: LogicalPosition };
/** The window's normal geometry, kept in session storage so a reload mid-stage can restore it. */
const SAVED_KEY = "gs.stage.saved";
let saved: Saved | null = null;
function remember(s: Saved | null) {
  saved = s;
  try {
    if (s) sessionStorage.setItem(SAVED_KEY, JSON.stringify({ w: s.size.width, h: s.size.height, x: s.pos.x, y: s.pos.y }));
    else sessionStorage.removeItem(SAVED_KEY);
  } catch {
    /* best effort */
  }
}
function recall(): Saved | null {
  try {
    const v = JSON.parse(sessionStorage.getItem(SAVED_KEY) ?? "null");
    return v ? { size: new LogicalSize(v.w, v.h), pos: new LogicalPosition(v.x, v.y) } : null;
  } catch {
    return null;
  }
}
/** Window calls are async; run them one transition at a time, in order. */
let queue: Promise<unknown> = Promise.resolve();
const serial = (job: () => Promise<void>) => {
  queue = queue.then(job).catch((e) => console.warn("stage window:", e));
  return queue;
};

/** Enter (or re-fit) stage mode for a board of the given aspect ratio (width / height). */
export function enterStage(aspect: number) {
  if (!IN_TAURI) return;
  return serial(async () => {
    const win = getCurrentWindow();
    const sf = await win.scaleFactor();
    const size = (await win.innerSize()).toLogical(sf);
    const pos = (await win.outerPosition()).toLogical(sf);
    const first = !saved;
    if (first) remember({ size, pos });

    setWindowTransparent(true);
    await win.setMinSize(STAGE_MIN);
    if (first) await setButtons(false);

    // Keep the current width (or the saved one when re-fitting) and take the board's shape,
    // staying inside the screen and centred where the window was.
    const mon = await currentMonitor();
    const maxW = mon ? (mon.workArea.size.width / mon.scaleFactor) * 0.96 : 1600;
    const maxH = mon ? (mon.workArea.size.height / mon.scaleFactor) * 0.9 : 900;
    let width = Math.min(first ? size.width : Math.max(size.width, size.height * aspect), maxW);
    let height = width / aspect;
    if (height > maxH) {
      height = maxH;
      width = height * aspect;
    }
    width = Math.round(width);
    height = Math.round(height);
    await win.setSize(new LogicalSize(width, height));
    await win.setPosition(
      new LogicalPosition(Math.round(pos.x + (size.width - width) / 2), Math.round(pos.y + (size.height - height) / 2)),
    );
  });
}

export function exitStage() {
  if (!IN_TAURI) return;
  return serial(async () => {
    const back = saved;
    remember(null);
    await restoreFrame();
    if (back) await place(back);
  });
}

const win = () => getCurrentWindow();
async function place(s: Saved) {
  await win().setSize(s.size);
  await win().setPosition(s.pos);
}
const setButtons = (visible: boolean) => invoke("set_window_buttons", { visible });

async function restoreFrame() {
  await setButtons(true);
  await win().setMinSize(NORMAL_MIN);
  setWindowTransparent(false);
}

/** A reload mid-stage would leave the window bare and small; put it back to normal. */
export function recoverFrame() {
  if (!IN_TAURI) return;
  return serial(async () => {
    await restoreFrame();
    const back = recall();
    remember(null);
    if (back) await place(back);
  });
}
