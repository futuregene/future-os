import type { ProvidersView } from "../../../integrations/agent/providers";
import { useCallback, useEffect, useState } from "react";
import { listAgentProviders } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

/**
 * App-wide provider availability. The onboarding gate in `AppShell` reads this to
 * decide whether to show the onboarding screen. Any usable provider (FutureOS
 * signed-in, builtin with a key, or custom provider) clears the gate. A user can
 * also skip the gate by choosing BYOK (`enableBYOK`), which hides the gate and
 * auto-opens Settings → Providers so they can add their own key without signing in.
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

  const enableBYOK = useCallback(() => {
    setByokMode(true);
  }, []);

  // True when any provider has a usable key: FutureOS signed in, a builtin with
  // a key, or any custom provider. Also true when the user chose BYOK.
  const hasProviders = byokMode || Boolean(
    (data?.builtin ?? []).some(p => p.hasApiKey)
    || (data?.custom ?? []).some(p => p.hasApiKey),
  );
  const initialLoading = loading && data === null;

  return { hasProviders, byokMode, enableBYOK, initialLoading };
}
