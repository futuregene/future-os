import { requestFutureModelsRefresh } from "./agentClient";

/**
 * How long one picker-open request suppresses the next.
 *
 * The platform catalogue changes at most a few times a day, so this window is
 * enough to keep a user who opens the picker repeatedly (or switches between the
 * composer and Settings → Models) from asking the platform every time, while a
 * model published minutes ago still shows up on the next open.
 */
export const MODEL_CATALOG_REFRESH_TTL_MS = 10 * 60 * 1000;

/** When the last request was made, or `undefined` before the first one. */
let lastRequestedAt: number | undefined;

/**
 * Ask the agent to refresh the Future catalogue in the background, at most once
 * per {@link MODEL_CATALOG_REFRESH_TTL_MS}.
 *
 * Call this when a model picker opens. It is deliberately fire-and-forget: the
 * picker shows the list it already has, the agent schedules the fetch and
 * answers immediately, and the refreshed list arrives through the agent's
 * `provider-config-changed` event (republished as `providers-changed`, which the
 * model list already refreshes on). Nothing here waits on the network, and a
 * failed request is swallowed — a picker is not the place to report that a
 * catalogue refresh did not happen.
 *
 * Returns whether a request was made, which is how tests observe the TTL gate.
 */
export function requestModelCatalogRefresh(now: number = Date.now()) {
  if (lastRequestedAt !== undefined && now - lastRequestedAt < MODEL_CATALOG_REFRESH_TTL_MS)
    return false;
  lastRequestedAt = now;
  void requestFutureModelsRefresh().catch(() => {});
  return true;
}

/** Test seam: forget the last request so each test starts from a cold window. */
export function resetModelCatalogRefreshForTests() {
  lastRequestedAt = undefined;
}
