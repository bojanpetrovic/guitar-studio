// Small React hooks shared across components.
import { RefObject, useEffect, useLayoutEffect, useRef } from "react";
import { meterLevel, Telemetry, useTelemetry } from "./api";

/** Close a popover on a click outside `root` or on Escape (while `open`). */
export function useDismiss(root: RefObject<HTMLElement | null>, open: boolean, close: () => void) {
  const latest = useRef(close);
  latest.current = close;
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => root.current && !root.current.contains(e.target as Node) && latest.current();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && latest.current();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, root]);
}

const SILENT: [number, number] = [0, 0];
/** How far a meter may fall per telemetry frame (~30 Hz). */
const FALL = 0.04;

/**
 * Stereo meter levels (0..1) for a telemetry peak, rising instantly and falling
 * smoothly, plus whether it clipped in the last 1.2 s. Render stays pure: the fall-off
 * state is committed after the render it was drawn in.
 */
export function usePeakLevels(select: (t: Telemetry | null) => [number, number] | undefined) {
  const peak = useTelemetry((t) => select(t) ?? SILENT);
  const shown = useRef<[number, number]>([0, 0]);
  const clippedAt = useRef(-Infinity);
  const levels: [number, number] = [
    Math.max(meterLevel(peak[0]), shown.current[0] - FALL),
    Math.max(meterLevel(peak[1]), shown.current[1] - FALL),
  ];
  const clipNow = peak[0] >= 0.999 || peak[1] >= 0.999;
  useLayoutEffect(() => {
    shown.current = levels;
    if (clipNow) clippedAt.current = performance.now();
  });
  const clipping = clipNow || performance.now() - clippedAt.current < 1200;
  return { levels, clipping };
}
