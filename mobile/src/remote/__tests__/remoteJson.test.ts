import { gzipSync, strToU8 } from "fflate";
import { decodeRemoteJson, decodeRemoteJsonAsync } from "../remoteJson";

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

  test("the cooperative decoder enforces the same ceiling", async () => {
    const oversized = new Uint8Array(1024 * 1024 + 1).fill(0x20);
    await expect(decodeRemoteJsonAsync(oversized)).rejects.toThrow("remote_json_too_large");
    await expect(decodeRemoteJsonAsync(new TextEncoder().encode("[1,2]"))).resolves.toEqual([1, 2]);
  });
});
