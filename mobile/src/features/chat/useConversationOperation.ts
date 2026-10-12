import { useCallback, useLayoutEffect, useMemo, useRef } from "react";
import type { useRemote } from "../../remote/RemoteContext";
import { useCommittedRef } from "../../lib/useCommittedRef";

type Remote = ReturnType<typeof useRemote>;

/** Navigation cancels UI work; an acknowledged new conversation keeps its send. */
export function useConversationOperation(remote: Remote) {
  const key = JSON.stringify([
    remote.credentials?.pairId, remote.credentials?.expectedDesktopId,
    remote.presence?.bridgeInstanceId, remote.selectedSessionId,
    remote.draft, remote.draftWorkspaceId,
  ]);
  // Each visit is distinct, including A → B → A navigation.
  const scope = useMemo(() => ({ key }), [key]);
  const lifetimeRef = useRef<object | null>(null);
  const mountedRef = useRef(false);
  const remoteCaptureRef = useCommittedRef(remote.captureConversationOperation);
  useLayoutEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);
  useLayoutEffect(() => {
    lifetimeRef.current = {};
    return () => { lifetimeRef.current = null; };
  }, [scope]);
  const captureOperation = useCallback(() => {
    const lifetime = lifetimeRef.current;
    const navigationIsCurrent = remoteCaptureRef.current?.();
    return () => mountedRef.current && (navigationIsCurrent
      ? navigationIsCurrent()
      : lifetime !== null && lifetimeRef.current === lifetime);
  }, [remoteCaptureRef]);
  return { scope, captureOperation };
}
