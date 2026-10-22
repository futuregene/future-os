/**
 * The composer's thinking picker offers the levels the *model* declares.
 *
 * `models.json` can narrow a model's vocabulary (`thinkingLevels`), and the agent
 * clamps every level outside it — so a menu that still listed all six would let
 * the user pick an effort the run immediately snaps away from. The keyboard ring
 * and the menu read one list (`modelThinkingLevels`), so both are checked here.
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
  thinkingLevels: ["off", "low", "high"],
  defaultThinkingLevel: "low",
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

/** The thinking trigger's visible label, then the rows its menu offers. */
function thinkingTrigger(): HTMLElement {
  const button = [...container.querySelectorAll<HTMLElement>("button")]
    .find(candidate => candidate.getAttribute("title") === "Thinking level");
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
  expect(menuRows()).toEqual(["Off", "Low", "High"]);
});

it("shows the stored level only while the model offers it", async () => {
  // `xhigh` came from another model (or from before this one narrowed its list):
  // the trigger must not claim an effort this model cannot run at.
  await mount([narrowModel], "xhigh");
  expect(thinkingTrigger().textContent).toContain("Low");
});

it("keeps all six when the model declares nothing", async () => {
  await mount([{ id: "narrow", provider: "custom", label: "Narrow" }], "medium");
  await act(async () => thinkingTrigger().click());
  expect(menuRows()).toEqual(["Off", "Minimal", "Low", "Medium", "High", "XHigh"]);
});
