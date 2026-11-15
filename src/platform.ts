// The computer the UI runs on. Shortcut hints are written the Mac way (⌘, ⇧, ⌫) and
// shown in this platform's words elsewhere; the shortcuts themselves take ⌘ or Ctrl.

const platform = navigator.platform || navigator.userAgent;
export const IS_MAC = /Mac|iPhone|iPad/.test(platform);
export const IS_WINDOWS = /Win/.test(platform);

/** "⇧⌘S" → "Ctrl+Shift+S", "⌘C / ⌘V" → "Ctrl+C / Ctrl+V", "⇧-click" → "Shift-click",
 *  "⌫" → "Delete" (not on a Mac). */
export function keys(text: string): string {
  if (IS_MAC) return text;
  return text
    .replace(/⇧⌘|⌘⇧/g, "Ctrl+Shift+")
    .replace(/⌘/g, "Ctrl+")
    .replace(/⇧(?=[\s-])/g, "Shift")
    .replace(/⇧/g, "Shift+")
    .replace(/⌫/g, "Delete");
}
