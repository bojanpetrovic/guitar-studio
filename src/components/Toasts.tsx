import { AlertTriangle, CheckCircle2, Info, Loader2, X } from "lucide-react";

export interface Toast {
  id: number;
  message: string;
  kind: "info" | "success" | "error" | "loading";
}

const icons = {
  info: <Info size={16} />,
  success: <CheckCircle2 size={16} />,
  error: <AlertTriangle size={16} />,
  loading: <Loader2 size={16} className="spin" />,
};

export default function Toasts({ toasts, onDismiss }: { toasts: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          {icons[t.kind]}
          <span>{t.message}</span>
          {t.kind !== "loading" && (
            <button className="icon-btn ghost" onClick={() => onDismiss(t.id)} aria-label="Dismiss">
              <X size={14} />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
