import { BaseEdge, EdgeLabelRenderer, EdgeProps, getBezierPath } from "@xyflow/react";
import { X } from "lucide-react";
import { useStudio } from "../App";

/** A patch cable: soft glow underneath, animated signal-flow dashes on top. */
export default function SignalEdge(props: EdgeProps) {
  const { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected, source, target } = props;
  const { actions } = useStudio();
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });

  return (
    <>
      <path d={path} className={`cable-glow ${selected ? "selected" : ""}`} />
      <BaseEdge path={path} className={`cable ${selected ? "selected" : ""}`} interactionWidth={22} />
      <path d={path} className="cable-flow" />
      <EdgeLabelRenderer>
        <button
          className={`cable-cut nodrag nopan ${selected ? "visible" : ""}`}
          style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          onClick={() => actions.disconnect(Number(source), Number(target))}
          title="Disconnect"
        >
          <X size={12} />
        </button>
      </EdgeLabelRenderer>
    </>
  );
}
