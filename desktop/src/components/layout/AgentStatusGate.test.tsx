// @vitest-environment jsdom
import type { AgentStatus, ShutdownProgress } from "../../integrations/agent/agentStatus";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { AgentStatusGate } from "./AgentStatusGate";

const mounted: Array<() => void> = [];

afterEach(() => {
  while (mounted.length)
    mounted.pop()?.();
});

function renderStatus(phase: AgentStatus["phase"], showWait = false, error?: string, shutdown?: ShutdownProgress, startup?: AgentStatus["startup"]) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(
    <AgentStatusGate showWait={showWait} status={{ phase, desktopVersion: "1.1.8", agentVersion: null, ...(error ? { error } : {}), ...(shutdown ? { shutdown } : {}), ...(startup ? { startup } : {}) }} />,
  ));
  mounted.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container.textContent ?? "";
}

describe("agent status gate", () => {
  it("shows cleanup stages and keeps terminal failures visible", () => {
    const progress: ShutdownProgress = { operationId: "stop-1", reason: "quit", state: "running", stage: "background", elapsedMs: 100, errors: [] };
    expect(renderStatus("stopping", false, undefined, progress)).toContain("Finishing background work");
    const failed = renderStatus("stopping", false, "deadline exceeded", { ...progress, state: "failed", errors: ["deadline exceeded"] });
    expect(failed).toContain("deadline exceeded");
    expect(failed).toContain("Updates, reset and restart are blocked");
    expect(failed).not.toContain("Future Agent could not start");
  });

  it("shows the connection gate reason for a blocked Agent", () => {
    expect(renderStatus("unavailable", false, "Agent instance lock is occupied")).toContain("Agent instance lock is occupied");
    expect(renderStatus("incompatible", false, "Agent serves a different data root")).toContain("Agent serves a different data root");
  });

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

  it("keeps showing progress while automatic recovery runs", () => {
    const text = renderStatus("recovering", true);
    expect(text).toContain("Please wait a moment.");
    expect(text).not.toContain("could not start");
  });

  it("reveals upgrade progress only after the waiting hint delay", () => {
    const progress = { stage: "upgrading_storage" as const, elapsedMs: 24000 };
    expect(renderStatus("starting", false, undefined, undefined, progress)).toBe("");
    const text = renderStatus("starting", true, undefined, undefined, progress);
    expect(text).toContain("Upgrading conversation data");
    expect(text).toContain("Startup is taking longer");
    expect(text).not.toContain("could not start");
  });

  it("describes missing status responses without claiming a startup failure", () => {
    const text = renderStatus("startup_timeout");
    expect(text).toContain("No startup status received for 15 seconds");
    expect(text).not.toContain("could not start");
  });

  it("shows restart and external Agent diagnostics for startup failures", () => {
    const text = renderStatus("spawn_failed");
    expect(text).toContain("Future Agent could not start");
    expect(text).toContain("Restart FutureOS");
    expect(text).toContain("external Agent");
    expect(text).toContain("availability and version");
  });

  it("shows the version mismatch guidance", () => {
    const text = renderStatus("incompatible");
    expect(text).toContain("does not match this FutureOS version");
    expect(text).toContain("Update or stop the external Agent");
    expect(text).toContain("cannot be bypassed");
  });
});
