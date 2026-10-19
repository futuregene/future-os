import { messageText, randomId } from "../codec";

describe("history text", () => {
  test("joins text blocks and ignores tool blocks", () => {
    expect(
      messageText([
        { kind: "text", text: "one" },
        { kind: "tool_call" },
        { kind: "text", text: "two" },
      ]),
    ).toBe("onetwo");
  });

  test("tolerates missing or null content (tool-call-only messages)", () => {
    expect(messageText(undefined)).toBe("");
    expect(messageText(null)).toBe("");
  });
});

describe("randomId", () => {
  test("prefixes a 128-bit hex random value", () => {
    const id = randomId("dev");
    expect(id).toMatch(/^dev_[0-9a-f]{32}$/);
  });
});
