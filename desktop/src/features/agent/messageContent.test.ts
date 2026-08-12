import { describe, expect, it } from "vitest";
import { attachmentInputs, stringifyMessageContent } from "./messageContent";

describe("stringifyMessageContent", () => {
  it("serializes text and attachments as a user_message envelope", () => {
    const parsed = JSON.parse(stringifyMessageContent("hi", [{ kind: "file", path: "/a", name: "a" } as never]));
    expect(parsed).toMatchObject({ type: "user_message", text: "hi" });
    expect(parsed.attachments).toHaveLength(1);
  });
});

describe("attachmentInputs", () => {
  it("maps kinds and includes thumbnails when present", () => {
    const inputs = attachmentInputs([
      { kind: "image", path: "/p.png", name: "p.png", thumbnail: "/t.png" },
      { kind: "file", path: "/f.pdf", name: "f.pdf" },
    ] as never[]);
    expect(inputs).toEqual([
      { path: "/p.png", kind: "image", name: "p.png", thumbnail: "/t.png" },
      { path: "/f.pdf", kind: "file", name: "f.pdf" },
    ]);
  });
});
