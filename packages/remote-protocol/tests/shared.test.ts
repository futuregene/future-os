import { describe, expect, test } from "vitest";
import { decodeRemoteJson, parsePairingInvitation } from "../src/index";
import type { HandshakeConfirmation, HandshakeRequest, RemoteCommand } from "../src/index";
import fixture from "../fixtures/protocol.json";

describe("shared Remote protocol fixtures", () => {
  test("requires a supported version, unique IDs and explicit parser differences", () => {
    expect(fixture.version).toBe(1);
    expect(fixture.cases.length).toBeGreaterThan(0);
    expect(new Set(fixture.cases.map(c => c.id)).size).toBe(fixture.cases.length);
    for (const c of fixture.cases) {
      if (c.kind === "invitation" && typeof c.expected === "object" && c.expected && "rust" in c.expected && "typescript" in c.expected && c.expected.rust !== c.expected.typescript) {
        expect(c.difference).toBeTruthy();
      }
    }
  });

  test.each(fixture.cases)("$id", c => {
    if (c.kind === "invitation") {
      expect(!!parsePairingInvitation(c.input as string)).toBe((c.expected as { typescript: boolean }).typescript);
    } else if (c.kind === "json") {
      const bytes = c.repeatSpace
        ? new TextEncoder().encode("0".padEnd(c.repeatSpace, " "))
        : new Uint8Array(c.bytes!);
      if (c.accepted) expect(decodeRemoteJson(bytes)).toEqual(c.expected);
      else if (c.error) expect(() => decodeRemoteJson(bytes)).toThrow(c.error);
      else expect(() => decodeRemoteJson(bytes)).toThrow();
    } else if (c.kind === "command") {
      // Client encoding preserves absent fields and nulls; the receiving Rust
      // decoder supplies its historical defaults. No JS server is introduced.
      const command = c.input as RemoteCommand;
      expect(decodeRemoteJson(new TextEncoder().encode(JSON.stringify(command)))).toEqual(c.input);
    } else if (c.kind === "handshake-request") {
      const request = c.input as HandshakeRequest;
      expect(decodeRemoteJson(new TextEncoder().encode(JSON.stringify(request)))).toEqual(c.input);
    } else if (c.kind === "handshake-confirmation") {
      const confirmation = decodeRemoteJson<HandshakeConfirmation>(new TextEncoder().encode(JSON.stringify(c.expected)));
      expect(confirmation).toEqual(c.expected);
      expect(confirmation.confirmed).toBe(true);
      expect(confirmation.presence.bridgeInstanceId).toBe(confirmation.bridgeInstanceId);
    } else {
      throw new Error(`Unsupported shared case ${c.id}`);
    }
  });
});
