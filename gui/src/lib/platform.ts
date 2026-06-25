/**
 * macOS draws the window "traffic-light" buttons in the top-left corner, so
 * titlebar UI (the sidebar toggle, the collapsed-panel header) must leave room
 * for them. Windows and Linux put window controls in the top-right, so the
 * top-left needs no inset.
 */
export const isMacOS
  = typeof navigator !== "undefined" && /Macintosh|Mac OS X/i.test(navigator.userAgent);
