/**
 * The polyfills stand in for the RN runtime's missing globals. `util.ts` is a
 * pure re-export (importing it is the whole behaviour); `randomBytes` is the
 * entropy source the pairing/crypto paths use, so its fill must come from the
 * platform CSPRNG rather than a predictable generator.
 */
import { TextDecoder, TextEncoder } from "../util";
import { randomBytes } from "../crypto";

test("the util polyfill forwards the runtime's own codecs", () => {
  expect(TextEncoder).toBe(globalThis.TextEncoder);
  expect(TextDecoder).toBe(globalThis.TextDecoder);
  expect(new TextEncoder().encode("中").length).toBe(3);
});

test("random bytes are the requested length and filled from the platform CSPRNG", () => {
  const fill = jest.spyOn(globalThis.crypto, "getRandomValues");
  const zero = randomBytes(0);
  expect(zero).toBeInstanceOf(Uint8Array);
  expect(zero.length).toBe(0);
  const bytes = randomBytes(32);
  expect(bytes.length).toBe(32);
  expect(fill).toHaveBeenCalledTimes(2);
  expect(fill.mock.calls[1]![0]).toBe(bytes);
});

test("two draws of the same length are not the same bytes", () => {
  expect(Array.from(randomBytes(16))).not.toEqual(Array.from(randomBytes(16)));
});
