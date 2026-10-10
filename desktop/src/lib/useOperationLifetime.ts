import { useCallback, useLayoutEffect, useRef } from "react";

/** Each async action belongs to one committed component and scope lifetime. */
export function useOperationLifetime(scope?: unknown) {
  const lifetimeRef = useRef<object | null>(null);
  useLayoutEffect(() => {
    lifetimeRef.current = {};
    return () => {
      lifetimeRef.current = null;
    };
  }, [scope]);

  return useCallback(() => {
    const lifetime = lifetimeRef.current;
    return () => lifetime !== null && lifetimeRef.current === lifetime;
  }, []);
}
