import { slopToTouchTarget } from "../../../theme/tokens";

/**
 * The transcript's folded controls, and the slop that lifts each to the platform
 * touch target.
 *
 * Both hug their text to stay out of the reader's way — the attachment chip is a
 * single 26px line, a step row a single 20px one — but each is the only way into
 * what it opens, so the tap area comes from `hitSlop` instead of the drawn box.
 *
 * They live here, apart from the components, because `react-native-web` does not
 * implement `hitSlop`: no capture can show the result, so the arithmetic has to
 * be readable and testable without a renderer. `foldedRowTouchTargets.test.ts`
 * holds both to `layout.touchTarget`.
 */
export const CHIP_HEIGHT = 26;
export const CHIP_HIT_SLOP = slopToTouchTarget(CHIP_HEIGHT);

export const STEP_ROW_HEIGHT = 20;
export const ROW_HIT_SLOP = slopToTouchTarget(STEP_ROW_HEIGHT);
