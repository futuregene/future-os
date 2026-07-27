import type { ProvidersView } from "../../../integrations/agent/providers";
import { useEffect } from "react";
import { FUTURE_PROVIDER_ID, listAgentProviders } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

/**
 * App-wide FutureOS sign-in state. The forced-login gate in `AppShell` reads
 * this to decide whether to overlay the login screen. `reload()` is wired to the
 * `future-auth-changed` event so a device login, sign-out, or hand-edited key
 * flips the gate without prop drilling. `useAsyncResource.reload` is silent
 * (data stays non-null), so those transitions never flash the neutral frame —
 * `initialLoading` is true only on the very first load.
 */
export function useFutureSignedIn() {
  const { data, loading, reload } = useAsyncResource<ProvidersView | null>(
    listAgentProviders,
    [],
    null,
  );

  useEffect(() => onFutureEvent("future-auth-changed", reload), [reload]);

  const signedIn = Boolean(
    data?.builtin.find(provider => provider.id === FUTURE_PROVIDER_ID)?.hasApiKey,
  );
  const initialLoading = loading && data === null;

  return { signedIn, initialLoading };
}
