// @vitest-environment jsdom
import type { StoredRun } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { RunError } from "./RunError";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(props: { errorMessage: string; errorType?: StoredRun["errorType"]; variant: "summary" | "banner" }) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => {
    root.render(<RunError {...props} />);
  });
  return container;
}

describe("runError", () => {
  it("shows the message and the typed label in the banner variant", () => {
    const container = mount({ errorMessage: "The stream ended unexpectedly.", errorType: "stream_disconnected", variant: "banner" });

    expect(container.textContent).toContain("The stream ended unexpectedly.");
    expect(container.textContent).toContain("Stream disconnected");
    // The banner box carries the danger frame and a label row above the message.
    const box = container.firstElementChild!;
    expect(box.className).toContain("border-danger-line");
    expect(box.className).toContain("p-2");
    expect(box.querySelector("svg")).not.toBeNull();
  });

  it("clamps the message in the compact summary variant and hides the icon row", () => {
    const container = mount({ errorMessage: "boom", variant: "summary" });

    const box = container.firstElementChild!;
    expect(box.className).toBe("mt-2");
    expect(box.querySelector("svg")).toBeNull();
    expect(box.querySelector("p")!.className).toContain("line-clamp-2");
  });

  it.each([
    ["command_failed", "Command failed"],
    ["model_failed", "Model failed"],
    ["abort_requested", "Aborted by user"],
    ["timeout", "Timeout"],
    ["unknown", "Unknown error"],
  ] as const)("labels %s as %j", (errorType, label) => {
    const container = mount({ errorMessage: "detail", errorType, variant: "summary" });

    expect(container.textContent).toContain(label);
  });

  it.each([undefined, null])("renders no label row for an absent error type (%j)", (errorType) => {
    const container = mount({ errorMessage: "no category", errorType: errorType as never, variant: "summary" });

    expect(container.textContent).toBe("no category");
    expect(container.querySelector("svg")).toBeNull();
  });

  it("ignores an error type the meta table does not know (forward-compatible payload)", () => {
    const container = mount({ errorMessage: "future kind", errorType: "hyperdrive_failed" as never, variant: "banner" });

    expect(container.textContent).toBe("future kind");
    expect(container.querySelector("svg")).toBeNull();
  });

  it("renders a long CJK message verbatim", () => {
    const message = "连接中断：远端在流式响应结束前关闭了连接，请重试。".repeat(20);
    const container = mount({ errorMessage: message, errorType: "stream_disconnected", variant: "banner" });

    expect(container.textContent).toContain(message);
  });
});
