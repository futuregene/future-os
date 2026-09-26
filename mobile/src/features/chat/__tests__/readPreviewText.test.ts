import { FileMode, type File } from "expo-file-system";
import { readPreviewText } from "../readPreviewText";
import { MARKDOWN_RENDER_BYTES, plainText } from "../utils";

jest.mock("expo-file-system", () => ({ FileMode: { ReadOnly: "r" } }));
jest.mock("../../../remote/files", () => ({
  TransferCancelledError: class extends Error { constructor() { super("transfer_cancelled"); } },
}));
function fixture(data: Uint8Array, reportedSize = data.length) {
  let offset = 0;
  const close = jest.fn();
  const readBytes = jest.fn((count: number) => {
    const part = data.subarray(offset, offset + count);
    offset += part.length;
    return part;
  });
  const open = jest.fn(() => ({ readBytes, close }));
  const bytes = jest.fn(() => { throw new Error("whole-file read is forbidden"); });
  return { file: { size: reportedSize, open, bytes } as unknown as File, open, readBytes, close, bytes,
    get totalRead() { return offset; } };
}
beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("a 10-MiB file reads only the preview budget, in bounded chunks, yielding to input", async () => {
  const f = fixture(new Uint8Array(MARKDOWN_RENDER_BYTES).fill(97), 10 * 1024 * 1024);
  const reading = readPreviewText(f.file);
  expect(f.totalRead).toBeLessThan(MARKDOWN_RENDER_BYTES);
  const otherTask = jest.fn();
  setTimeout(otherTask, 0);
  await jest.runAllTimersAsync();
  const result = await reading;
  expect(result).toEqual({ text: "a".repeat(MARKDOWN_RENDER_BYTES), truncated: true });
  expect(f.totalRead).toBe(MARKDOWN_RENDER_BYTES);
  expect(Math.max(...f.readBytes.mock.calls.map(([count]) => count))).toBeLessThanOrEqual(64 * 1024);
  expect(otherTask).toHaveBeenCalled();
  expect(f.bytes).not.toHaveBeenCalled();
  expect(f.open).toHaveBeenCalledWith(FileMode.ReadOnly);
  expect(f.close).toHaveBeenCalledTimes(1);
});

test("truncation does not invent a replacement character or reject a split UTF-8 code point", async () => {
  const data = new Uint8Array(MARKDOWN_RENDER_BYTES).fill(97);
  data[data.length - 1] = 0xe4; // First byte of a CJK code point, continued outside the preview.
  const f = fixture(data, data.length + 2);
  const reading = readPreviewText(f.file);
  await jest.runAllTimersAsync();
  expect(await reading).toEqual({ text: "a".repeat(data.length - 1), truncated: true });
  const completeButInvalid = fixture(new Uint8Array([0xe4]));
  expect(await readPreviewText(completeButInvalid.file)).toBeNull();
});

test.each([new Uint8Array([0]), new Uint8Array([0xff]), new Uint8Array([65, 1, 66]),
  new Uint8Array([0xc0, 0x80]), new Uint8Array([0xed, 0xa0, 0x80]), new Uint8Array([0xf4, 0x90, 0x80, 0x80])])("binary/invalid input is not displayed as text", async data => {
  const f = fixture(data);
  expect(await readPreviewText(f.file)).toBeNull();
  expect(f.close).toHaveBeenCalledTimes(1);
});

test("cancellation after a yield closes the handle and prevents more reads", async () => {
  const f = fixture(new Uint8Array(MARKDOWN_RENDER_BYTES).fill(97));
  const controller = new AbortController();
  const reading = readPreviewText(f.file, controller.signal);
  const rejected = expect(reading).rejects.toThrow("transfer_cancelled");
  const count = f.readBytes.mock.calls.length;
  controller.abort();
  await jest.runAllTimersAsync();
  await rejected;
  expect(f.readBytes).toHaveBeenCalledTimes(count);
  expect(f.close).toHaveBeenCalledTimes(1);
});

test("validation works with the mobile decoder fallback that has no fatal/stream support", () => {
  const NativeDecoder = global.TextDecoder;
  global.TextDecoder = class {
    constructor(_label?: string, options?: { fatal?: boolean }) {
      if (options?.fatal) throw new Error("fatal unsupported");
    }
    decode(bytes: Uint8Array, options?: { stream?: boolean }) {
      if (options?.stream) throw new Error("stream unsupported");
      return new NativeDecoder().decode(bytes);
    }
  } as unknown as typeof TextDecoder;
  try {
    const bytes = new TextEncoder().encode("中文😀");
    expect(plainText(bytes)).toBe("中文😀");
    expect(plainText(bytes.subarray(0, bytes.length - 1), true)).toBe("中文");
    expect(plainText(bytes.subarray(0, bytes.length - 1))).toBeNull();
  } finally { global.TextDecoder = NativeDecoder; }
});

test("unexpected EOF fails without leaking the handle", async () => {
  const f = fixture(new Uint8Array([65]), 100);
  await expect(readPreviewText(f.file)).rejects.toThrow("preview_read_size_mismatch");
  expect(f.close).toHaveBeenCalledTimes(1);
});

test("a file whose reported length is not a usable size is refused before any read", async () => {
  // A negative or non-numeric size cannot be turned into a byte count; reading
  // anyway would either throw inside the native layer or inline the whole file.
  await expect(readPreviewText(fixture(new Uint8Array([65]), -1).file))
    .rejects.toThrow("invalid_preview_size");
  const nan = fixture(new Uint8Array([65]), Number.NaN);
  await expect(readPreviewText(nan.file)).rejects.toThrow("invalid_preview_size");
  expect(nan.open).not.toHaveBeenCalled();
});
