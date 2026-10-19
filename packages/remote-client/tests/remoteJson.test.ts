import { decodeRemoteJsonAsync } from "../src/remoteJson";

describe("cooperative remote JSON", () => {
  test("the cooperative decoder enforces the same ceiling", async () => {
    const oversized = new Uint8Array(1024 * 1024 + 1).fill(0x20);
    await expect(decodeRemoteJsonAsync(oversized)).rejects.toThrow("remote_json_too_large");
    await expect(decodeRemoteJsonAsync(new TextEncoder().encode("[1,2]"))).resolves.toEqual([1, 2]);
  });
});
