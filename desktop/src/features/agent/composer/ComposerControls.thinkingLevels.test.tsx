/**
 * The composer's thinking picker offers exactly the levels the *model* declares.
 *
 * A current agent publishes the model's effective list — always starting with
 * the implicit `default`, arbitrary wire values included — and the agent clamps
 * every level outside it. The keyboard ring and the menu read that one list
 * (`modelThinkingLevels`), so both are checked here; a list with a single entry
 * is the only case that has no choice to offer.
 */
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { resetShortcutScopes } from "../../keyboard/scopes";
import { resetAllCommandBindings } from "../../keyboard/shortcuts";
import { ComposerControls } from "./ComposerControls";

vi.mock("../../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => {
    if (command === "list_agent_providers")
      return { builtin: [], custom: [] };
    return [];
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  resetAllCommandBindings();
  resetShortcutScopes();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const narrowModel = {
  id: "narrow",
  provider: "custom",
  label: "Narrow",
  thinkingLevels: ["default", "off", "low", "high"],
  defaultThinkingLevel: "default",
};

function mount(modelOptions: Parameters<typeof ComposerControls>[0]["modelOptions"], thinkingLevel: string) {
  return act(async () => root.render(
    <ComposerControls
      approvalTier="off"
      attachmentCount={0}
      futureSessionStatus="ready"
      handleAttachFiles={async () => {}}
      inputEmpty
      modelId="custom/narrow"
      modelOptions={modelOptions}
      modelsEmptyReason={undefined}
      onThinkingLevelChange={() => {}}
      sendPending={false}
      recommendPending={false}
      compactionPending={false}
      thinkingLevel={thinkingLevel}
    />,
  ));
}

/** The thinking trigger (its aria-label never changes, unlike its tooltip). */
function thinkingTrigger(): HTMLButtonElement {
  const button = [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(candidate => candidate.getAttribute("aria-label") === "Thinking level");
  expect(button, "the thinking trigger").not.toBeNull();
  return button!;
}

function menuRows(): string[] {
  // The rows are plain buttons inside the shared menu surface (they carry no
  // ARIA role of their own).
  const panel = container.querySelector(".shadow-panel");
  if (!panel)
    return [];
  return [...panel.querySelectorAll("button")].map(row => row.textContent?.trim() ?? "");
}

it("lists only the levels the model declares", async () => {
  await mount([narrowModel], "low");
  await act(async () => thinkingTrigger().click());
  expect(menuRows()).toEqual(["default", "off", "low", "high"]);
});

it("offers arbitrary wire values verbatim, with a raw label fallback", async () => {
  await mount([{ ...narrowModel, thinkingLevels: ["default", "max", "ultra"], defaultThinkingLevel: "default" }], "max");
  await act(async () => thinkingTrigger().click());
  expect(menuRows()).toEqual(["default", "max", "ultra"]);
});

it("shows the stored level only while the model offers it", async () => {
  // `xhigh` came from another model (or from before this one narrowed its list):
  // the trigger must not claim an effort this model cannot run at.
  await mount([narrowModel], "xhigh");
  expect(thinkingTrigger().textContent).toContain("default");
});

it("keeps all six when the model declares nothing", async () => {
  await mount([{ id: "narrow", provider: "custom", label: "Narrow" }], "medium");
  await act(async () => thinkingTrigger().click());
  expect(menuRows()).toEqual(["off", "minimal", "low", "medium", "high", "xhigh"]);
});

it("disables the picker when the model offers only the implicit default", async () => {
  await mount([{ id: "narrow", provider: "custom", label: "Narrow", thinkingLevels: ["default"], defaultThinkingLevel: "default" }], "default");
  expect(thinkingTrigger().disabled).toBe(true);
  expect(thinkingTrigger().textContent).toContain("default");
});

it("enables the picker as soon as there is a second option", async () => {
  await mount([{ ...narrowModel, thinkingLevels: ["default", "off"], defaultThinkingLevel: "default" }], "default");
  expect(thinkingTrigger().disabled).toBe(false);
});
