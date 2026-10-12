import { useEffect, useRef, useState } from "react";

export function useMinimumVisible(active: boolean, minimumMs: number, key: string) {
  const [presentation, setPresentation] = useState({ key, visible: active });
  const shownAtRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const visible = presentation.key === key && presentation.visible;

  useEffect(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    if (active) {
      if (!visible || !shownAtRef.current) shownAtRef.current = Date.now();
      if (!visible) {
        // The notice must become visible in the same status transition; delaying
        // this update would allow a fast sync to finish before it is ever shown.
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setPresentation({ key, visible: true });
      }
      return;
    }
    if (!visible) return;

    const remaining = Math.max(0, shownAtRef.current + minimumMs - Date.now());
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      shownAtRef.current = 0;
      setPresentation(current =>
        current.key === key ? { key, visible: false } : current,
      );
    }, remaining);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = null;
    };
  }, [active, key, minimumMs, visible]);

  return visible;
}
