/**
 * Width floors for the three-column shell. The conversation area (center) is
 * the priority: the side panels yield to it rather than squeezing it, and they
 * stop being columns when their floor no longer fits (see `canShow*Panel`).
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
 * Whether the right context panel can be a *column* beside the conversation.
 * Deliberately measured against all three floors (not against the rail's live
 * width): the column must not silently squeeze the conversation or force the
 * rail to move. Below this the panel is not hidden — it opens as an overlay
 * over the conversation instead (the shell's drawer; see `AppShell`), which is
 * why losing the column must not take the affording control with it.
 */
export function canShowRightPanel(windowWidth: number): boolean {
  return (
    windowWidth
    >= MIN_LEFT_PANEL_WIDTH + MIN_CENTER_PANEL_WIDTH + MIN_RIGHT_PANEL_WIDTH
  );
}
