import { useRef } from "react";
import { meterLevel, useTelemetry } from "../api";

const EMPTY: [number, number] = [0, 0];

/** Stereo peak meter with smooth fall-off. `id` = node id, or "master". */
export default function Meter({ id, vertical = false }: { id: number | "master"; vertical?: boolean }) {
  const peak = useTelemetry((t) =>
    !t ? EMPTY : id === "master" ? t.outputPeak : (t.nodes.find((n) => n.id === id)?.peak ?? EMPTY),
  );
  const shown = useRef<[number, number]>([0, 0]);
  const clip = useRef(0);

  const next: [number, number] = [0, 1].map((c) => {
    const target = meterLevel(peak[c]);
    return Math.max(target, shown.current[c] - 0.04);
  }) as [number, number];
  shown.current = next;
  if (peak[0] >= 0.999 || peak[1] >= 0.999) clip.current = Date.now();
  const clipping = Date.now() - clip.current < 1200;

  return (
    <div className={`meter ${vertical ? "vertical" : ""} ${clipping ? "clip" : ""}`}>
      {next.map((v, i) => (
        <div className="meter-lane" key={i}>
          <div className="meter-fill" style={vertical ? { height: `${v * 100}%` } : { width: `${v * 100}%` }} />
        </div>
      ))}
    </div>
  );
}
