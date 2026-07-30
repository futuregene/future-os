import type { ProvidersView } from "../../../integrations/agent/providers";
import { useCallback, useEffect, useRef, useState } from "react";
import { FUTURE_PROVIDER_ID, listAgentProviders } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

/**
 * App-wide provider availability. The onboarding gate in `AppShell` reads this to
 * decide whether to show the onboarding screen. Any usable provider (FutureOS
 * signed-in, builtin with a key, or custom provider) clears the gate — but only
 * after the post-login initialization runs (models + skills + agent ready).
 * A user can also skip the gate by choosing BYOK (`enableBYOK`), which hides the
 * gate and auto-opens Settings → Providers so they can add their own key without
 * signing in.
 *
 * `reload()` is wired to `future-auth-changed` so the gate flips without
 * prop drilling. `useAsyncResource.reload` is silent (data stays non-null), so
 * transitions never flash the neutral frame — `initialLoading` is true only on
 * the very first load.
 */
export function useHasProviders() {
  const { data, loading, reload } = useAsyncResource<ProvidersView | null>(
    listAgentProviders,
    [],
    null,
  );

  useEffect(() => onFutureEvent("future-auth-changed", reload), [reload]);

  const [byokMode, setByokMode] = useState(false);
  // Remains true while the post-login initialization runs (models + skills +
  // agent readiness). The gate stays up until finishInit() is called, then the
  // provider re-check lets the normal app through.
  const [initPending, setInitPending] = useState(false);

  // Track whether FutureOS had a key BEFORE the latest event, so we can detect a
  // fresh sign-in (the key goes from absent → present across a reload).
  const hadFutureKeyRef = useRef(false);
  useEffect(() => {
    if (data) {
      const hasFutureKey = data.builtin.some(
        p => p.id === FUTURE_PROVIDER_ID && p.hasApiKey,
      );
      // Fresh sign-in detected: key appeared after not being present.
      if (hasFutureKey && !hadFutureKeyRef.current) {
        setInitPending(true);
      }
      hadFutureKeyRef.current = hasFutureKey;
    }
  }, [data]);

  const enableBYOK = useCallback(() => {
    setByokMode(true);
  }, []);

  const finishInit = useCallback(() => {
    setInitPending(false);
  }, []);

  // True when any provider has a usable key: FutureOS signed in, a builtin with
  // a key, or any custom provider. Also true when the user chose BYOK.
  const hasProviders = byokMode || Boolean(
    (data?.builtin ?? []).some(p => p.hasApiKey)
    || (data?.custom ?? []).some(p => p.hasApiKey),
  );
  const initialLoading = loading && data === null;

  // The gate shows when:
  // - initial probe is still running
  // - no provider is usable yet
  // - post-login initialization is still pending (key was just added, init is
  //   running inside OnboardingGate)
  const showGate = initialLoading || !hasProviders || initPending;

  return { showGate, byokMode, enableBYOK, finishInit, initialLoading };
}
