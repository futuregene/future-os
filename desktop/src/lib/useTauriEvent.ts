import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

/**
 * Subscribe to a Tauri backend event for the component's lifetime. Registers a
 * listener once per `event` name and always invokes the latest `handler` (held
 * in a ref, so a changing closure doesn't churn the subscription), and tears the
 * listener down on unmount — the async `listen()` unlisten is always awaited and
 * called, so no listener leaks.
 */
export function useTauriEvent<T = unknown>(event: string, handler: (payload: T) => void) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    const unlisten = listen<T>(event, e => handlerRef.current(e.payload));
    return () => {
      void unlisten.then(stop => stop());
    };
  }, [event]);
}
