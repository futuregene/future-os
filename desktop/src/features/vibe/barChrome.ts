/**
 * The panel's own top bar, as geometry: where its content may start, and what may drag the
 * window.
 *
 * The window is a macOS **overlay title bar** (`titleBarStyle: "Overlay"`,
 * `trafficLightPosition: { x: 14, y: 26 }` in `tauri.conf.json`), so the traffic lights
 * are drawn *over* the page at roughly `x 14–72, y 19–33`. Every surface that draws at the
 * top-left reserves that box: the rail puts its toggle at `left-20` (80px), the collapsed
 * thread header pads its toggle by `pl-16`. The Vibe panel is `fixed inset-0`, i.e. it is
 * the only surface that starts at x=0 with text in it, so it has to make the same
 * reservation itself.
 *
 * A pure function rather than an inline ternary because the numbers are the point: they
 * come from the config, and getting them wrong is exactly the bug this fixes (the panel's
 * label was drawn under the lights).
 */

/**
 * How far the bar's content starts from the left edge.
 *
 * 80px clears the lights' right edge (~72px) with room for the 14px gap the rest of the
 * bar uses. `pl-3` is the ordinary 12px inset, which is correct everywhere the lights are
 * not drawn — a non-macOS window, and macOS fullscreen, where the system hides them.
 */
export function barStart(reserveTrafficLights: boolean): string {
  return reserveTrafficLights ? "pl-20" : "pl-3";
}

/** Whether this window, right now, has macOS traffic lights over the page's top-left. */
export function reserveTrafficLights(isMac: boolean, isFullscreen: boolean): boolean {
  return isMac && !isFullscreen;
}

/**
 * The bar's height — and the reason it is a constant rather than a class in the markup.
 *
 * Every other top bar in the app is `h-12`: the rail's header, the thread header, the
 * context panel's, the new-conversation header, the collapsed titlebar toggle. The panel's
 * bar was `h-9`, so its text centred at 18px while those surfaces — and the traffic lights
 * they sit beside — centre at 24px. A 6px lift reads exactly as the report did: "the text
 * after the dots is not aligned". Matching the height is the whole fix; adding a top
 * padding instead would have introduced a second number to keep in sync with the first.
 */
export const BAR_HEIGHT = "h-12";
