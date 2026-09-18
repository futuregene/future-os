// @vitest-environment jsdom
import type { AgentStatus } from "../../integrations/agent/agentStatus";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { AgentStatusGate } from "./AgentStatusGate";

const mounted: Array<() => void> = [];

afterEach(() => {
  while (mounted.length)
    mounted.pop()?.();
});

function renderStatus(phase: AgentStatus["phase"], showWait = false) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(
    <AgentStatusGate showWait={showWait} status={{ phase, desktopVersion: "1.1.8", agentVersion: null }} />,
  ));
  mounted.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container.textContent ?? "";
}

describe("agent status gate", () => {
  it("shows only the animation during the first five seconds", () => {
    const text = renderStatus("starting");
    expect(text).toBe("");
    expect(text).not.toContain("Future Agent is starting");
    expect(text).not.toContain("Sign in");
  });

  it("adds the waiting hint after five seconds", () => {
    const text = renderStatus("starting", true);
    expect(text).toContain("Please wait a moment.");
  });

  it("shows restart and reinstall guidance for startup failures", () => {
    const text = renderStatus("spawn_failed");
    expect(text).toContain("Future Agent could not start");
    expect(text).toContain("Restart FutureOS");
    expect(text).toContain("reinstall FutureOS");
  });

  it("shows the version mismatch guidance", () => {
    const text = renderStatus("incompatible");
    expect(text).toContain("does not match this FutureOS version");
    expect(text).toContain("finish the update");
  });
});
