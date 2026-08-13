import {
  decodePairingCode,
  jwtExpiry,
  MAX_PROMPT_MESSAGE_BYTES,
  messageText,
  pairingCodeFromQr,
  parsePairingInvitation,
  randomId,
  utf8Bytes,
} from "../codec";

function base64Url(value: unknown): string {
  return globalThis
    .btoa(JSON.stringify(value))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

describe("pairing codec", () => {
  test("decodes a current v2 code", () => {
    const code = base64Url({
      v: 2,
      nonce: "rpn_test",
      claim_url: "https://test.future-os.cn/client/v1/remote/pair/claim",
      exp: 200,
    });
    expect(decodePairingCode(code, 100)?.nonce).toBe("rpn_test");
  });

  test("rejects expired and wrong-version codes", () => {
    expect(
      decodePairingCode(
        base64Url({ v: 2, nonce: "x", claim_url: "https://example.com", exp: 99 }),
        100,
      ),
    ).toBeNull();
    expect(
      decodePairingCode(
        base64Url({ v: 1, nonce: "x", claim_url: "https://example.com", exp: 200 }),
        100,
      ),
    ).toBeNull();
  });

  test("extracts custom-scheme QR payload", () => {
    const invitation = "futureos://remote/pair?code=abc_123&desktopId=desktop_123&desktopKey=UABC";
    expect(pairingCodeFromQr(invitation)).toBe(invitation);
    expect(parsePairingInvitation(invitation)).toEqual({
      code: "abc_123",
      desktopId: "desktop_123",
      desktopPublicKey: "UABC",
    });
  });

  test("rejects invitations without a bound desktop identity", () => {
    expect(pairingCodeFromQr("futureos://remote/pair?code=abc_123")).toBeNull();
    expect(pairingCodeFromQr("raw-code")).toBeNull();
  });

  test("rejects a malformed invitation URL instead of throwing", () => {
    // A non-URL (or URL without a parseable host) makes `new URL` throw — the
    // parser must degrade to null rather than leak a TypeError to the caller.
    expect(parsePairingInvitation("not a url")).toBeNull();
    expect(parsePairingInvitation("%%%")).toBeNull();
  });
});

describe("history text", () => {
  test("joins text blocks and ignores tool blocks", () => {
    expect(
      messageText([
        { type: "text", text: "one" },
        { type: "tool_use" },
        { type: "text", text: "two" },
      ]),
    ).toBe("onetwo");
  });

  test("tolerates missing or null content (tool-call-only messages)", () => {
    expect(messageText(undefined)).toBe("");
    expect(messageText(null)).toBe("");
  });
});

describe("prompt payload budget", () => {
  test("utf8Bytes counts multi-byte characters as bytes, not code units", () => {
    expect(utf8Bytes("")).toBe(0);
    expect(utf8Bytes("hello")).toBe(5);
    // ASCII chars: 1 byte each.
    expect(utf8Bytes("你")).toBe(3); // CJK → 3 UTF-8 bytes.
    expect(utf8Bytes("𝄞")).toBe(4); // astral plane → 4 bytes (2 UTF-16 units).
  });

  test("a maximum-size prompt still fits the NATS wire budget", () => {
    // 512KB of ASCII fits under the 1MB relay cap with room for the envelope.
    expect(MAX_PROMPT_MESSAGE_BYTES).toBe(512 * 1024);
    const prompt = "a".repeat(MAX_PROMPT_MESSAGE_BYTES);
    expect(utf8Bytes(prompt)).toBe(MAX_PROMPT_MESSAGE_BYTES);
    expect(utf8Bytes(prompt)).toBeLessThan(900 * 1024);
  });
});

describe("randomId", () => {
  test("prefixes a 128-bit hex random value", () => {
    const id = randomId("dev");
    expect(id).toMatch(/^dev_[0-9a-f]{32}$/);
  });
});

describe("jwt expiry", () => {
  function encodeSegment(claims: unknown): string {
    return globalThis
      .btoa(JSON.stringify(claims))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
  }

  test("reads a valid exp", () => {
    const jwt = `h.${encodeSegment({ exp: 1_800_000_000 })}.s`;
    expect(jwtExpiry(jwt)).toBe(1_800_000_000);
  });

  test("returns null instead of 0 for a missing/undecodable expiry", () => {
    // The desktop rejects a JWT with no readable exp outright; mobile must not
    // treat the failure as "already expired" (which would drive a 5s refresh storm).
    expect(jwtExpiry("")).toBeNull();
    expect(jwtExpiry("no.segments")).toBeNull();
    expect(jwtExpiry("h.bogus.s")).toBeNull();
    expect(jwtExpiry(`h.${encodeSegment({})}.s`)).toBeNull();
    expect(jwtExpiry(`h.${encodeSegment({ exp: "later" })}.s`)).toBeNull();
    expect(jwtExpiry(`h.${encodeSegment({ exp: 0 })}.s`)).toBeNull();
  });
});
