// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ shellProps: null as unknown }));

vi.mock("../components/layout/AppShell", async () => {
  const { createElement: create } = await import("react");
  return {
    AppShell: (props: unknown) => {
      mocks.shellProps = props;
      return create("div", { "data-child": "app-shell" });
    },
  };
});

describe("app root", () => {
  it("mounts the application shell", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(createElement(App)));

    expect(container.querySelector("[data-child=\"app-shell\"]")).not.toBeNull();
    // The root takes no props of its own — everything the shell needs it reads
    // from its own hooks, so a stray prop here would be a wiring mistake.
    expect(mocks.shellProps).toEqual({});
    act(() => root.unmount());
    container.remove();
  });
});
