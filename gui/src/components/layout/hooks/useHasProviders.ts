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
 * The gate can also be triggered from the Settings page (Providers/Account
 * "Connect" / "Sign in" buttons) via the `show-onboarding` event, which reuses
 * the same login + init flow.
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
  // True while the post-login init runs (models + skills + agent readiness).
  const [initPending, setInitPending] = useState(false);
  // True when the gate was explicitly requested from Settings (Providers or
  // Account page "Connect" / "Sign in" buttons). Cleared when init finishes or
  // BYOK / cancel is used.
  const [forceOnboarding, setForceOnboarding] = useState(false);

  useEffect(() => onFutureEvent("show-onboarding", () => {
    setForceOnboarding(true);
  }), []);

  // Track whether FutureOS had a key BEFORE the latest data snapshot, so we can
  // detect a fresh sign-in (key goes absent → present). `firstDataRef` prevents
  // the very first data load (user already logged in from a previous session)
  // from being misidentified as a fresh sign-in.
  const hadFutureKeyRef = useRef(false);
  const firstDataRef = useRef(true);
  useEffect(() => {
    if (data) {
      const hasFutureKey = data.builtin.some(
        p => p.id === FUTURE_PROVIDER_ID && p.hasApiKey,
      );
      if (firstDataRef.current) {
        // Seed the ref from the first load so we don't false-trigger init.
        firstDataRef.current = false;
        hadFutureKeyRef.current = hasFutureKey;
      }
      else if (hasFutureKey && !hadFutureKeyRef.current) {
        // Fresh sign-in detected across a reload.
        setInitPending(true);
      }
      hadFutureKeyRef.current = hasFutureKey;
    }
  }, [data]);

  const enableBYOK = useCallback(() => {
    setByokMode(true);
    setInitPending(false);
    setForceOnboarding(false);
  }, []);

  const finishInit = useCallback(() => {
    setInitPending(false);
    setForceOnboarding(false);
  }, []);

  // Cancel the reconnect flow without entering BYOK mode. Clears the
  // force-onboarding flag, any pending init, and the BYOK bypass so the gate
  // returns to its initial state when no actual provider exists.
  const cancelLogin = useCallback(() => {
    setByokMode(false);
    setInitPending(false);
    setForceOnboarding(false);
  }, []);

  // True when any provider has a usable key: FutureOS signed in, a builtin with
  // a key, or any custom provider. Also true when the user chose BYOK.
  const hasProviders = byokMode || Boolean(
    (data?.builtin ?? []).some(p => p.hasApiKey)
    || (data?.custom ?? []).some(p => p.hasApiKey),
  );

  // Whether the provider data shows at least one key (regardless of BYOK mode).
  const hasAnyProvider = Boolean(
    (data?.builtin ?? []).some(p => p.hasApiKey)
    || (data?.custom ?? []).some(p => p.hasApiKey),
  );

  const initialLoading = loading && data === null;

  // The gate shows when:
  // - initial probe is still running
  // - no provider is usable yet (first-launch onboarding)
  // - post-login initialization is still pending
  // - explicitly requested from Settings (reconnect flow)
  const showGate = initialLoading || !hasProviders || initPending || forceOnboarding;

  return { showGate, byokMode, enableBYOK, finishInit, cancelLogin, hasAnyProvider, forceOnboarding, initialLoading };
}
