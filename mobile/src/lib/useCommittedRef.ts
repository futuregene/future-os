import { useLayoutEffect, useRef } from "react";

/** Event callbacks read the latest committed state, including batched renders. */
export function useCommittedRef<T>(value: T) {
  const ref = useRef(value);
  useLayoutEffect(() => { ref.current = value; }, [value]);
  return ref;
}
