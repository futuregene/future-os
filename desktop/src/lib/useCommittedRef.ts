import { useLayoutEffect, useRef } from "react";

/** A stable ref for callbacks that must only read committed props or state. */
export function useCommittedRef<T>(value: T) {
  const ref = useRef(value);
  useLayoutEffect(() => {
    ref.current = value;
  }, [value]);
  return ref;
}
