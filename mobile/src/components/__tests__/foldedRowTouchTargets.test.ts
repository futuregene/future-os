import { layout, slopToTouchTarget } from "../../theme/tokens";
import { CHIP_HEIGHT, CHIP_HIT_SLOP, ROW_HIT_SLOP, STEP_ROW_HEIGHT } from "../../features/chat/messages/touchTargets";

// Two controls in the transcript hug their text to stay out of the reader's way,
// so their tap area comes from `hitSlop` rather than from the drawn box:
//
//   the attachment chip   26px line, the only way into a file from a message
//   the step rows         20px line, the only way into the run behind it
//
// `react-native-web` does not implement `hitSlop` (an `elementFromPoint` probe
// reports the drawn box), so neither a screenshot nor the web harness can show
// the result — a device is the only place it is observable. These assertions are
// therefore the evidence: they hold the arithmetic to the platform minimum, so a
// change to either drawn height cannot quietly shrink the target again.

describe("a folded row still reaches the platform touch target", () => {
  test("the attachment chip", () => {
    expect(CHIP_HEIGHT + CHIP_HIT_SLOP.top + CHIP_HIT_SLOP.bottom).toBeGreaterThanOrEqual(layout.touchTarget);
    // Its own line is 26, so nine a side is what the floor leaves over.
    expect(CHIP_HIT_SLOP).toEqual({ top: 9, bottom: 9 });
  });

  test("the step rows", () => {
    expect(STEP_ROW_HEIGHT + ROW_HIT_SLOP.top + ROW_HIT_SLOP.bottom).toBeGreaterThanOrEqual(layout.touchTarget);
    // 8 a side only reached 36 and measured short of the floor.
    expect(ROW_HIT_SLOP).toEqual({ top: 12, bottom: 12 });
  });

  test("a drawn row already at the floor is left alone", () => {
    expect(slopToTouchTarget(layout.touchTarget)).toEqual({ top: 0, bottom: 0 });
    expect(slopToTouchTarget(layout.touchTarget + 10)).toEqual({ top: 0, bottom: 0 });
  });

  test("an odd gap rounds up, never down", () => {
    // 44 - 27 = 17 over two sides is 8.5; rounding down would land at 43.
    const slop = slopToTouchTarget(27);
    expect(27 + slop.top + slop.bottom).toBeGreaterThanOrEqual(layout.touchTarget);
  });
});
