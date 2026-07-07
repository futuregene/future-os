/**
 * macOS draws the window "traffic-light" buttons in the top-left corner, so
 * titlebar UI (the sidebar toggle, the collapsed-panel header) must leave room
 * for them. Windows and Linux put window controls in the top-right, so the
 * top-left needs no inset.
 */
export const isMacOS
  = typeof navigator !== "undefined" && /Macintosh|Mac OS X/i.test(navigator.userAgent);

/**
 * Platform detection for user-facing labels that follow OS conventions — e.g.
 * "Reveal in Finder" (macOS) vs "Show in File Explorer" (Windows).
 */
export const isWindows
  = typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
