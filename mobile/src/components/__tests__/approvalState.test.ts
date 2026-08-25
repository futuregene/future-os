import { approvalDecisionDisabled } from "../approvalState";

describe("approvalDecisionDisabled", () => {
  test("keeps malformed capabilities rejectable but blocks approval", () => {
    expect(approvalDecisionDisabled(false, true)).toEqual({
      approved: true,
      rejected: false,
    });
  });

  test("blocks both decisions while a response is being submitted", () => {
    expect(approvalDecisionDisabled(true, false)).toEqual({
      approved: true,
      rejected: true,
    });
  });

  test("allows both decisions for a valid idle request", () => {
    expect(approvalDecisionDisabled(false, false)).toEqual({
      approved: false,
      rejected: false,
    });
  });
});
