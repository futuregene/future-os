/**
 * Opening the model picker is also the moment to ask for a fresh catalogue: a
 * model published since the agent started must show up without a restart, and
 * asking must never block the picker that is opening.
 */
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { resetModelCatalogRefreshForTests } from "../../../integrations/agent/modelCatalogRefresh";
import { invokeCommand } from "../../../integrations/tauri/invoke";
import { resetShortcutScopes } from "../../keyboard/scopes";
import { resetAllCommandBindings } from "../../keyboard/shortcuts";
import { ComposerControls } from "./ComposerControls";

vi.mock("../../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => {
    if (command === "list_agent_providers")
      return { builtin: [], custom: [] };
    return { requested: true };
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  resetAllCommandBindings();
  resetShortcutScopes();
  resetModelCatalogRefreshForTests();
  vi.mocked(invokeCommand).mockClear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const models = [
  { id: "one", provider: "custom", label: "One" },
  { id: "two", provider: "custom", label: "Two" },
];

/** The model trigger, found the way a reader finds it: its title. */
function modelTrigger(): HTMLElement {
  const button = [...container.querySelectorAll<HTMLElement>("button")]
    .find(candidate => candidate.getAttribute("title") === "Model");
  expect(button, "the model trigger").not.toBeNull();
  return button!;
}

async function mount() {
  await act(async () => root.render(
    <ComposerControls
      approvalTier="off"
      attachmentCount={0}
      futureSessionStatus="ready"
      handleAttachFiles={async () => {}}
      inputEmpty
      modelId="custom/one"
      modelOptions={models}
      modelsEmptyReason={undefined}
      onModelChange={() => {}}
      onThinkingLevelChange={() => {}}
      sendPending={false}
      recommendPending={false}
      compactionPending={false}
      thinkingLevel="off"
    />,
  ));
}

it("asks for a background catalogue refresh when the picker opens", async () => {
  await mount();
  expect(invokeCommand).not.toHaveBeenCalledWith("request_future_models_refresh");

  await act(async () => modelTrigger().click());
  expect(invokeCommand).toHaveBeenCalledWith("request_future_models_refresh");
});

it("does not ask again when the picker is merely closed", async () => {
  await mount();
  await act(async () => modelTrigger().click());
  await act(async () => modelTrigger().click());

  const requests = vi.mocked(invokeCommand).mock.calls.filter(([command]) => command === "request_future_models_refresh");
  expect(requests).toHaveLength(1);
});
