import { describe, expect, test } from "vitest";
import { gzipSync, strToU8 } from "fflate";
import {
  BINARY_FRAME_MAGIC, decodeRemoteJson, remoteReplyBody, splitBinaryFrame,
} from "../src/json";

/** Write the frame the host writes: magic, big-endian header length, header, body. */
function frameBytes(header: Uint8Array, body: Uint8Array): Uint8Array {
  const frame = new Uint8Array(BINARY_FRAME_MAGIC.length + 4 + header.length + body.length);
  frame.set(BINARY_FRAME_MAGIC, 0);
  new DataView(frame.buffer).setUint32(BINARY_FRAME_MAGIC.length, header.length, false);
  frame.set(header, BINARY_FRAME_MAGIC.length + 4);
  frame.set(body, BINARY_FRAME_MAGIC.length + 4 + header.length);
  return frame;
}

describe("remote JSON encoding", () => {
  test("decodes legacy plain JSON", () => {
    const encoded = strToU8('{"ok":true}');
    expect(decodeRemoteJson<{ ok: boolean }>(encoded)).toEqual({ ok: true });
  });

  test("decodes an automatically compressed standard gzip payload", () => {
    const source = JSON.stringify({ entries: ["repeated ".repeat(10_000)] });
    const encoded = gzipSync(strToU8(source), { level: 1 });
    expect(decodeRemoteJson<{ entries: string[] }>(encoded)).toEqual(JSON.parse(source));
  });

  test("rejects a malformed gzip payload instead of treating it as JSON", () => {
    const encoded = new Uint8Array([0x1f, 0x8b, 1]);
    expect(() => decodeRemoteJson(encoded)).toThrow();
  });

  test("rejects a gzip payload whose footer exceeds the JSON allocation cap", () => {
    const encoded = new Uint8Array([0x1f, 0x8b, 1, 0, 16, 0]);
    expect(() => decodeRemoteJson(encoded)).toThrow("remote_json_gzip_too_large");
  });

  test("a plain reply over the cap is refused before it is parsed or copied", () => {
    const oversized = new Uint8Array(1024 * 1024 + 1).fill(0x20);
    expect(() => decodeRemoteJson(oversized)).toThrow("remote_json_too_large");
    // A payload exactly at the cap is still legal.
    expect(decodeRemoteJson(new TextEncoder().encode('{"ok":true}'))).toEqual({ ok: true });
  });

  test("a binary frame keeps its header and its bytes apart", () => {
    const header = strToU8('{"success":true,"data":{"readChunk":{"id":"read_a"}}}');
    const body = new Uint8Array([0, 255, 123, 0x7b]);
    const frame = frameBytes(header, body);
    const reply = remoteReplyBody(frame);
    expect(reply.body).toEqual(body);
    expect(decodeRemoteJson(reply.json)).toEqual({
      success: true, data: { readChunk: { id: "read_a" } },
    });
  });

  test("the frame is recognised through gzip, and legacy pages are not frames", () => {
    const header = strToU8('{"success":true}');
    const body = new Uint8Array(192 * 1024).fill(0x41);
    const compressed = gzipSync(frameBytes(header, body), { level: 1 });
    expect(remoteReplyBody(compressed).body).toEqual(body);

    // A base64 page still parses as a document, with no carried bytes.
    const legacy = strToU8('{"success":true,"data":{"readChunk":{"data":"AAAA"}}}');
    expect(remoteReplyBody(legacy).body).toBeUndefined();
    expect(splitBinaryFrame(legacy)).toBeNull();
  });

  test("a frame whose header length overruns its body is refused", () => {
    const overrun = frameBytes(strToU8("{}"), new Uint8Array());
    // Claim one byte more than the frame holds.
    overrun[7] = 3;
    expect(() => splitBinaryFrame(overrun)).toThrow("remote_binary_frame_invalid");
    expect(() => splitBinaryFrame(BINARY_FRAME_MAGIC)).toThrow("remote_binary_frame_invalid");
  });

});
