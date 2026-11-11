import { Volume2, VolumeX } from "lucide-react";
import { useOutputMute } from "../studio";

/** Mutes / unmutes the main output (title bar, phone header). */
export default function MuteButton({ className = "" }: { className?: string }) {
  const { muted, toggle } = useOutputMute();
  return (
    <button
      className={`icon-btn mute-btn ${muted ? "on" : ""} ${className}`}
      onClick={toggle}
      aria-pressed={muted}
      title={muted ? "Output muted — click to unmute" : "Mute the output"}
    >
      {muted ? <VolumeX size={16} /> : <Volume2 size={16} />}
    </button>
  );
}
