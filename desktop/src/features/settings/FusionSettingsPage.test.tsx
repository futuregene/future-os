// @vitest-environment jsdom
import type { FusionSettings } from "@future-os/thread-projection";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setLanguage } from "../../i18n";
import { FusionSettingsPage } from "./FusionSettingsPage";
import { useUnsavedGuardHost } from "./unsavedGuard";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

const MODELS: AgentModelOption[] = [
  { id: "deepseek-flash", label: "DeepSeek Flash", provider: "future", thinkingLevels: ["default", "medium", "high"] },
  { id: "glm-5.3-flash", label: "GLM 5.3 Flash", provider: "future", thinkingLevels: ["default", "medium", "high"] },
];

function role(model: string, thinking = "high") {
  return { model: `future/${model}`, thinking_level: thinking };
}

/** Three participants: a removal is the cheapest edit that changes the draft. */
const SETTINGS: FusionSettings = {
  enabled: true,
  revision: 3,
  configuration: {
    version: 2,
    moderator: role("deepseek-flash"),
    panel: [role("deepseek-flash"), role("glm-5.3-flash"), role("deepseek-flash", "medium")],
    step_timeout_secs: 120,
    run_timeout_secs: 600,
  },
};

function makeSource(
  save = vi.fn(async (patch: { configuration?: FusionSettings["configuration"]; enabled?: boolean }) => ({
    ...SETTINGS,
    enabled: patch.enabled ?? SETTINGS.enabled,
    configuration: patch.configuration ?? SETTINGS.configuration,
    revision: SETTINGS.revision + 1,
  })),
) {
  return {
    load: async () => SETTINGS,
    save,
    models: async () => MODELS,
    providers: async () => ({ builtin: [], custom: [] }),
  };
}

type Source = ReturnType<typeof makeSource>;

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  setLanguage("en");
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

/** The surface that owns navigation: it guards its own "leave" action. */
function Harness({ onLeave, source }: { onLeave: () => void; source: Source }) {
  const guard = useUnsavedGuardHost();
  const [leaves, setLeaves] = useState(0);
  return (
    <>
      <button
        data-leave
        onClick={() => guard.api.confirm(() => {
          setLeaves(value => value + 1);
          onLeave();
        })}
        type="button"
      >
        leave
      </button>
      <span data-leaves={leaves} />
      <FusionSettingsPage guard={guard.api} source={source} />
      {guard.dialog}
    </>
  );
}

async function flush() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

async function mount(source: Source = makeSource(), onLeave = vi.fn()) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(<Harness onLeave={onLeave} source={source} />);
  });
  await flush();
  return { container, onLeave, source };
}

function button(container: HTMLElement, label: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.trim() === label)!;
}

async function click(container: HTMLElement, label: string) {
  await act(async () => {
    button(container, label).click();
  });
  await flush();
}

function dialogTitle(container: HTMLElement) {
  return [...container.querySelectorAll("[role=dialog] h2")].map(item => item.textContent);
}

function saveAction(container: HTMLElement) {
  return button(container, "Save model configuration") as HTMLButtonElement;
}

function removeParticipant(container: HTMLElement, index: number) {
  return container.querySelector<HTMLButtonElement>(`[aria-label="Remove Participant ${index}"]`)!;
}

describe("fusion settings save bar", () => {
  it("keeps the save action in a bar that says where the draft stands", async () => {
    const { container } = await mount();

    const save = saveAction(container);
    expect(save.disabled).toBe(true);
    expect(save.closest(".sticky") !== null).toBe(true);
    expect(container.textContent).toContain("Changes apply only after you save.");
  });

  it("enables the action and says the draft is unsaved as soon as it changes", async () => {
    const { container } = await mount();

    await act(async () => removeParticipant(container, 3).click());
    await flush();

    expect(saveAction(container).disabled).toBe(false);
    expect(container.textContent).toContain("Unsaved changes");
    expect(container.textContent).not.toContain("Changes apply only after you save.");
  });
});

describe("fusion settings unsaved guard", () => {
  it("leaves without a prompt when nothing was edited", async () => {
    const { container, onLeave } = await mount();

    await click(container, "leave");

    expect(onLeave).toHaveBeenCalledTimes(1);
    expect(dialogTitle(container)).toEqual([]);
  });

  it("asks before dropping a draft, and staying keeps it", async () => {
    const { container, onLeave, source } = await mount();

    await act(async () => removeParticipant(container, 3).click());
    await flush();
    await click(container, "leave");

    expect(dialogTitle(container)).toEqual(["Save your changes?"]);
    expect(container.textContent).toContain("Fusion model configuration has edits you haven't saved.");
    expect(onLeave).not.toHaveBeenCalled();

    await click(container, "Keep editing");

    expect(dialogTitle(container)).toEqual([]);
    expect(onLeave).not.toHaveBeenCalled();
    // The draft survived the question.
    expect(saveAction(container).disabled).toBe(false);
    expect(source.save).not.toHaveBeenCalled();
  });

  it("saves from the prompt and only then leaves", async () => {
    const { container, onLeave, source } = await mount();

    await act(async () => removeParticipant(container, 3).click());
    await flush();
    await click(container, "leave");
    await click(container, "Save and leave");

    expect(source.save).toHaveBeenCalledTimes(1);
    expect((source.save.mock.calls[0]![0] as { configuration?: { panel: unknown[] } }).configuration?.panel).toHaveLength(2);
    expect(onLeave).toHaveBeenCalledTimes(1);
    expect(dialogTitle(container)).toEqual([]);
  });

  it("leaves without saving when the draft is discarded", async () => {
    const { container, onLeave, source } = await mount();

    await act(async () => removeParticipant(container, 3).click());
    await flush();
    await click(container, "leave");
    await click(container, "Discard changes");

    expect(source.save).not.toHaveBeenCalled();
    expect(onLeave).toHaveBeenCalledTimes(1);
    expect(dialogTitle(container)).toEqual([]);
  });

  it("stays on the page when the save the prompt asked for fails", async () => {
    const save = vi.fn(async () => {
      throw new Error("host refused");
    });
    const { container, onLeave } = await mount(makeSource(save));

    await act(async () => removeParticipant(container, 3).click());
    await flush();
    await click(container, "leave");
    await click(container, "Save and leave");

    expect(onLeave).not.toHaveBeenCalled();
    expect(dialogTitle(container)).toEqual([]);
    expect(container.textContent).toContain("host refused");
  });
});
