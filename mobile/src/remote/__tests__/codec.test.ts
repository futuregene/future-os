import {
  decodePairingCode,
  messageText,
  pairingCodeFromQr,
  parsePairingInvitation,
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
});
