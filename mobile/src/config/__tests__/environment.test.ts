import { natsWsUrlScheme } from "../environment";

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
