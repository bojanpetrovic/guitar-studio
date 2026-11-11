import { usePeakLevels } from "../hooks";

/** Stereo peak meter with smooth fall-off. `id` = node id, or "master". */
export default function Meter({ id, vertical = false }: { id: number | "master"; vertical?: boolean }) {
  const { levels: next, clipping } = usePeakLevels((t) =>
    id === "master" ? t?.outputPeak : t?.nodes.find((n) => n.id === id)?.peak,
  );

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
