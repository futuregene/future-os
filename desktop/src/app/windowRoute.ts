/**
 * Which window a webview is, from its URL.
 *
 * A pure function over the query string rather than a read of
 * `window.location` inline, because the decision is what puts a *native window*
 * on one page or the other: parsed wrongly, a settings window renders the whole
 * app shell (and a desktop's settings page is then unreachable), or the main
 * window renders a settings page for a machine nobody asked about.
 */
export type WindowRoute
  = | { kind: "app" }
    | { kind: "peer-settings"; desktopId: string };

export function windowRoute(search: string): WindowRoute {
  const params = new URLSearchParams(search);
  if (params.get("window") !== "peer-settings")
    return { kind: "app" };
  // An id is required: a settings window without one has nothing to show, and
  // rendering the app shell instead would silently ignore the URL the backend
  // built.
  const desktopId = params.get("desktop") ?? "";
  if (desktopId === "")
    return { kind: "app" };
  return { kind: "peer-settings", desktopId };
}
