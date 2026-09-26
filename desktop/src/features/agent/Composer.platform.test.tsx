import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

/**
 * The platform-conditional copy in the approval-tier menu.
 *
 * `Composer` describes the sandbox tier differently per OS (`isWindows` /
 * `isLinux` come from `src/lib/platform.ts`, which reads `navigator.userAgent`)
 * and separately while the sandbox probe is still running. On any one machine
 * only one of those arms is reachable, which is why a branch audit found them
 * uncovered; the fix is to pin the platform module rather than the user agent,
 * so every arm is exercised in a single run.
 *
 * This is the subtree's real `platform-cfg` evidence - not "N/A by
 * construction", but a platform-dependent decision pinned by mocking the module
 * that detects the platform.
 */
const platform = vi.hoisted(() => ({ isLinux: false, isWindows: false }));
vi.mock("../../lib/platform", () => ({
  // Getters, not plain properties: the factory runs once at import, so a plain
  // `isWindows: platform.isWindows` would snapshot `false` and the mocked
  // export could never change (which is how the first version of the Windows
  // test below passed nothing and failed on the copy).
  get isLinux() {
    return platform.isLinux;
  },
  get isMacOS() {
    return false;
  },
  get isWindows() {
    return platform.isWindows;
  },
}));

const availability = vi.hoisted(() => ({
  value: { available: true, definitive: true, resolved: true },
}));
vi.mock("../../integrations/agent/useSandboxAvailability", () => ({
  useSandboxAvailability: () => availability.value,
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const MODELS = [
  { id: "m1", label: "Model One", provider: "future", supportsImages: true, supportsThinkingLevels: ["off"] },
] as unknown as AgentModelOption[];

let container: HTMLDivElement;
let root: Root;

function render() {
  act(() => root.render(
    <Composer
      approvalTier="sandbox"
      modelId="m1"
      modelOptions={MODELS}
      onChangeApprovalTier={vi.fn()}
      onSend={vi.fn(async () => {})}
    />,
  ));
}

beforeEach(() => {
  platform.isLinux = false;
  platform.isWindows = false;
  availability.value = { available: true, definitive: true, resolved: true };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function openTierMenu() {
  // The tier control is a `SelectMenu` trigger: a `button[title]`, not an
  // `aria-label` (only the attach/thinking/stop controls use aria-labels).
  const trigger = [...container.querySelectorAll<HTMLButtonElement>("button[title]")]
    .find(candidate => candidate.getAttribute("title") === "Approval mode");
  await act(async () => {
    trigger!.click();
  });
}

describe("approval-tier description by platform", () => {
  it("says it is still checking while the sandbox probe has not resolved", async () => {
    // boundary: the arm that does not depend on the platform at all - it uses
    // the hook's own `resolved` flag, so no OS is faked to reach it.
    availability.value = { available: false, definitive: false, resolved: false };
    render();
    await openTierMenu();

    expect(container.textContent).toContain("Checking the system sandbox");
  });

  it("describes the sandbox tier the Windows way on Windows", async () => {
    platform.isWindows = true;
    render();
    await openTierMenu();

    expect(container.textContent).toContain("Restricts out-of-scope writes and asks when needed");
  });

  it("describes the sandbox tier the generic way on Linux", async () => {
    platform.isLinux = true;
    render();
    await openTierMenu();

    // Honest limit: in English `approvalTierDesc.sandboxLinux` and
    // `approvalTierDesc.sandbox` are the SAME string ("Runs sandboxed and asks
    // when needed"), so this cannot distinguish the Linux arm from the
    // fallback by copy alone. It still exercises the arm - `isLinux` is a live
    // `true` here - and the Windows test above is the one that separates the
    // arms, because its copy differs from the fallback.
    expect(container.textContent).toContain("Runs sandboxed and asks when needed");
    expect(container.textContent).not.toContain("Restricts out-of-scope writes");
  });
});
