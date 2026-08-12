// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { copyText } from "./clipboard";

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

// jsdom does not implement execCommand — install a controllable stub.
function stubExecCommand(result: boolean) {
  const exec = vi.fn().mockReturnValue(result);
  Object.defineProperty(document, "execCommand", { value: exec, configurable: true });
  return exec;
}

describe("copyText", () => {
  it("does nothing for an empty value", async () => {
    const exec = stubExecCommand(true);
    await copyText("");
    expect(exec).not.toHaveBeenCalled();
  });

  it("copies via execCommand and cleans up the textarea", async () => {
    const exec = stubExecCommand(true);
    await copyText("hello");
    expect(exec).toHaveBeenCalledWith("copy");
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("falls back to the async clipboard API when execCommand fails", async () => {
    stubExecCommand(false);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await copyText("hello");
    expect(writeText).toHaveBeenCalledWith("hello");
  });

  it("throws when neither path can copy", async () => {
    stubExecCommand(false);
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    await expect(copyText("hello")).rejects.toThrow("Copy failed");
  });
});
