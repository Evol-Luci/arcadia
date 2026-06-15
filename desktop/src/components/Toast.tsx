import { useEffect } from "react";
import { useStore } from "../store/useStore";

export function Toast() {
  const toast = useStore((s) => s.toast);
  const setToast = useStore((s) => s.setToast);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 3500);
    return () => clearTimeout(t);
  }, [toast, setToast]);

  if (!toast) return null;
  return (
    <div className="pointer-events-none fixed bottom-6 left-1/2 z-50 -translate-x-1/2 animate-fade-up">
      <div className="glass rounded-full border-primary/40 px-5 py-2.5 text-sm font-semibold shadow-glow">
        {toast}
      </div>
    </div>
  );
}
