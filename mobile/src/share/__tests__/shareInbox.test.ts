import {
  markShareLanded,
  shareLandedRevision,
  subscribeShareLanded,
} from "../shareInbox";

test("bumps the revision and notifies subscribers as shares land", () => {
  const before = shareLandedRevision();
  const seen: number[] = [];
  const unsubscribe = subscribeShareLanded(() => seen.push(shareLandedRevision()));

  markShareLanded();
  expect(shareLandedRevision()).toBe(before + 1);
  markShareLanded();
  expect(shareLandedRevision()).toBe(before + 2);
  expect(seen).toEqual([before + 1, before + 2]);

  unsubscribe();
  markShareLanded();
  expect(seen).toHaveLength(2);
});
