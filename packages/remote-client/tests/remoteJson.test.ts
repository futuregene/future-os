import { BINARY_FRAME_MAGIC } from "@future-os/remote-protocol";
import { decodeRemoteJsonAsync, decodeRemoteReplyAsync } from "../src/remoteJson";

/** Write the frame the host writes: magic, big-endian header length, header, body. */
function frameBytes(header: string, body: Uint8Array): Uint8Array {
  const encoded = new TextEncoder().encode(header);
  const frame = new Uint8Array(BINARY_FRAME_MAGIC.length + 4 + encoded.length + body.length);
  frame.set(BINARY_FRAME_MAGIC, 0);
  new DataView(frame.buffer).setUint32(BINARY_FRAME_MAGIC.length, encoded.length, false);
  frame.set(encoded, BINARY_FRAME_MAGIC.length + 4);
  frame.set(body, BINARY_FRAME_MAGIC.length + 4 + encoded.length);
  return frame;
}

describe("cooperative remote JSON", () => {
  test("the cooperative decoder enforces the same ceiling", async () => {
    const oversized = new Uint8Array(1024 * 1024 + 1).fill(0x20);
    await expect(decodeRemoteJsonAsync(oversized)).rejects.toThrow("remote_json_too_large");
    await expect(decodeRemoteJsonAsync(new TextEncoder().encode("[1,2]"))).resolves.toEqual([1, 2]);
  });

  test("a negotiated page's bytes come out beside its reply, unseen by equality", async () => {
    const body = new Uint8Array([0, 1, 2, 250]);
    const reply = await decodeRemoteReplyAsync<{ success: boolean; data: unknown }>(
      frameBytes('{"success":true,"data":{"readChunk":{"id":"read_a"}}}', body));
    expect(reply.data).toEqual({ readChunk: { id: "read_a" } });
    expect((reply as { chunkBytes?: Uint8Array }).chunkBytes).toEqual(body);
    // Non-enumerable: spreading or comparing a reply must stay the wire document
    // it always was, or every existing equality assertion would need the bytes.
    expect(Object.keys(reply)).toEqual(["success", "data"]);
    expect({ ...reply }).not.toHaveProperty("chunkBytes");
  });

  test("an ordinary reply carries no bytes at all", async () => {
    const reply = await decodeRemoteReplyAsync<{ success: boolean }>(
      new TextEncoder().encode('{"success":true}'));
    expect(reply).toEqual({ success: true });
    expect(reply).not.toHaveProperty("chunkBytes");
  });
});
