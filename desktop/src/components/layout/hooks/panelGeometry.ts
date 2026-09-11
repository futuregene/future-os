/**
 * Width floors for the three-column shell. The conversation area (center) is
 * the priority: the side panels yield to it rather than squeezing it, and they
 * collapse entirely when their floor no longer fits (see `canShow*Panel`).
 */
export const MIN_LEFT_PANEL_WIDTH = 224;
export const MIN_CENTER_PANEL_WIDTH = 384;
export const MIN_RIGHT_PANEL_WIDTH = 384;

/**
 * Whether the left rail can stay expanded. Below this the rail folds into its
 * hover overlay, so the center keeps its floor instead of being squeezed by a
 * fixed 224px column.
 */
export function canShowLeftPanel(windowWidth: number): boolean {
  return windowWidth >= MIN_LEFT_PANEL_WIDTH + MIN_CENTER_PANEL_WIDTH;
}

/**
 * Whether the right context panel can open. Deliberately measured against all
 * three floors (not against the rail's live width): opening the panel is a
 * user action that must not silently squeeze the conversation or force the
 * rail to move, so below this the panel stays closed.
 */
export function canShowRightPanel(windowWidth: number): boolean {
  return (
    windowWidth
    >= MIN_LEFT_PANEL_WIDTH + MIN_CENTER_PANEL_WIDTH + MIN_RIGHT_PANEL_WIDTH
  );
}
