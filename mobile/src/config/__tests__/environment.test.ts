import { isExpectedClaimUrl, natsWsUrlScheme } from "../environment";

describe("isExpectedClaimUrl", () => {
  test.each(["future-os.cn", "test.future-os.cn"])("accepts the %s platform from the QR", host => {
    expect(isExpectedClaimUrl(`https://${host}/client/v1/remote/pair/claim`)).toBe(true);
  });

  test.each([
    "http://future-os.cn/client/v1/remote/pair/claim",
    "https://future-os.cn.example.com/client/v1/remote/pair/claim",
    "https://example.com/client/v1/remote/pair/claim",
    "https://future-os.cn:8443/client/v1/remote/pair/claim",
    "https://future-os.cn@evil.example/client/v1/remote/pair/claim",
    "https://user:password@future-os.cn/client/v1/remote/pair/claim",
    "https://future-os.cn/prefix/client/v1/remote/pair/claim",
    "https://future-os.cn/client/v1/remote/auth/token",
    "https://future-os.cn/client/v1/remote/pair/claim?redirect=other",
    "https://future-os.cn/client/v1/remote/pair/claim#other",
    "futureos://future-os.cn/client/v1/remote/pair/claim",
    "/client/v1/remote/pair/claim",
    "not a url",
  ])("rejects an untrusted or malformed claim URL: %s", url => {
    expect(isExpectedClaimUrl(url)).toBe(false);
  });
});

describe("natsWsUrlScheme", () => {
  test("accepts wss:// endpoints", () => {
    expect(natsWsUrlScheme("wss://test.future-os.cn:9090")).toBe("wss");
    expect(natsWsUrlScheme("wss://nats.future-os.cn")).toBe("wss");
    expect(natsWsUrlScheme("WSS://test.future-os.cn:9090")).toBe("wss");
  });

  test("recognizes ws:// as insecure", () => {
    expect(natsWsUrlScheme("ws://test.future-os.cn:9090")).toBe("ws");
  });

  test("rejects malformed or non-websocket URLs", () => {
    expect(natsWsUrlScheme("tls://test.future-os.cn:4222")).toBe("other");
    expect(natsWsUrlScheme("not a url")).toBe("other");
  });
});
