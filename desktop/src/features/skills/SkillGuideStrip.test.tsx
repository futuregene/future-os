// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { SkillGuideStrip } from "./SkillGuideStrip";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  fetchCoachPrompt: vi.fn(),
  fetchSkillManualUrl: vi.fn(),
  openExternalUrl: vi.fn(),
}));

vi.mock("./skillGuidePrompt", () => ({
  fetchCoachPrompt: () => mocks.fetchCoachPrompt(),
  fetchSkillManualUrl: () => mocks.fetchSkillManualUrl(),
}));
vi.mock("../../integrations/storage/files", () => ({ openExternalUrl: (...args: unknown[]) => mocks.openExternalUrl(...args) }));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.fetchCoachPrompt.mockReset();
  mocks.fetchCoachPrompt.mockResolvedValue("teach me");
  mocks.fetchSkillManualUrl.mockReset();
  mocks.fetchSkillManualUrl.mockResolvedValue("https://docs.example.com/manual");
  mocks.openExternalUrl.mockReset();
  mocks.openExternalUrl.mockResolvedValue(undefined);
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(onStartCoachConversation = vi.fn().mockResolvedValue(undefined)) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => {
    root.render(<SkillGuideStrip onStartCoachConversation={onStartCoachConversation} />);
  });
  return { container, onStartCoachConversation };
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.includes(text)) as HTMLButtonElement | undefined;
}

function popover(container: HTMLElement) {
  return container.querySelector("[class*='shadow-panel']");
}

/** Collect `toast` events emitted while `run` executes. */
async function toasts(run: () => Promise<void>) {
  const seen: { message: string; tone?: string }[] = [];
  const off = onFutureEvent("toast", detail => seen.push(detail));
  try {
    await run();
  }
  finally {
    off();
  }
  return seen;
}

describe("skillGuideStrip popover", () => {
  it("starts collapsed and toggles the hint popover", () => {
    const { container } = mount();

    expect(popover(container)).toBeNull();
    expect(button(container, "How to use skills")).toBeTruthy();

    act(() => button(container, "How to use skills")!.click());
    expect(popover(container)).not.toBeNull();
    expect(popover(container)!.textContent).toContain("Type / in the chat input");

    act(() => button(container, "How to use skills")!.click());
    expect(popover(container)).toBeNull();
  });

  it("collapses on Escape and on an outside click", () => {
    const first = mount();
    act(() => button(first.container, "How to use skills")!.click());
    expect(popover(first.container)).not.toBeNull();
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(popover(first.container)).toBeNull();

    const second = mount();
    act(() => button(second.container, "How to use skills")!.click());
    expect(popover(second.container)).not.toBeNull();
    act(() => {
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    });
    expect(popover(second.container)).toBeNull();
  });
});

describe("skillGuideStrip coach action", () => {
  it("fetches the prompt, starts the conversation and collapses", async () => {
    const { container, onStartCoachConversation } = mount();
    act(() => button(container, "How to use skills")!.click());

    await act(async () => {
      button(container, "Teach me to use skills")!.click();
    });

    expect(mocks.fetchCoachPrompt).toHaveBeenCalledTimes(1);
    expect(onStartCoachConversation).toHaveBeenCalledExactlyOnceWith("teach me");
    expect(popover(container)).toBeNull();
  });

  it("toasts the failure and keeps the popover open when the prompt cannot be fetched", async () => {
    mocks.fetchCoachPrompt.mockRejectedValue(new Error("platform unreachable"));
    const { container, onStartCoachConversation } = mount();
    act(() => button(container, "How to use skills")!.click());

    const emitted = await toasts(async () => {
      await act(async () => {
        button(container, "Teach me to use skills")!.click();
      });
    });

    expect(emitted).toHaveLength(1);
    expect(emitted[0]!.tone).toBe("error");
    expect(emitted[0]!.message).toContain("platform unreachable");
    expect(onStartCoachConversation).not.toHaveBeenCalled();
    // The user can retry from the still-open popover.
    expect(popover(container)).not.toBeNull();
    expect(button(container, "Teach me to use skills")!.disabled).toBe(false);
  });

  it("leaves reporting to the caller when starting the conversation fails, but clears the busy state", async () => {
    const onStartCoachConversation = vi.fn().mockRejectedValue(new Error("create failed"));
    const { container } = mount(onStartCoachConversation);
    act(() => button(container, "How to use skills")!.click());

    const emitted = await toasts(async () => {
      await act(async () => {
        button(container, "Teach me to use skills")!.click();
      });
    });

    // The caller owns its own toast; this component must not double-report.
    expect(emitted).toEqual([]);
    expect(popover(container)).toBeNull();
    act(() => button(container, "How to use skills")!.click());
    expect(button(container, "Teach me to use skills")!.disabled).toBe(false);
  });

  it("does not open the manual while a coach start is in flight", async () => {
    let resolvePrompt!: (value: string) => void;
    mocks.fetchCoachPrompt.mockReturnValue(new Promise((resolve) => {
      resolvePrompt = resolve;
    }));
    const { container, onStartCoachConversation } = mount();
    act(() => button(container, "How to use skills")!.click());

    await act(async () => {
      button(container, "Teach me to use skills")!.click();
    });
    expect(button(container, "Teach me to use skills")!.disabled).toBe(true);
    // The manual button stays enabled, so its handler's guard is the exclusion.
    expect(button(container, "User manual")!.disabled).toBe(false);

    await act(async () => {
      button(container, "User manual")!.click();
    });

    expect(mocks.fetchSkillManualUrl).not.toHaveBeenCalled();
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();

    await act(async () => {
      resolvePrompt("teach me");
    });
    expect(onStartCoachConversation).toHaveBeenCalledExactlyOnceWith("teach me");
  });
});

describe("skillGuideStrip manual action", () => {
  it("opens the manual URL in the browser and collapses", async () => {
    const { container } = mount();
    act(() => button(container, "How to use skills")!.click());

    await act(async () => {
      button(container, "User manual")!.click();
    });

    expect(mocks.openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://docs.example.com/manual");
    expect(popover(container)).toBeNull();
  });

  it("reports an informational toast when the platform has no manual for this language", async () => {
    mocks.fetchSkillManualUrl.mockResolvedValue(null);
    const { container } = mount();
    act(() => button(container, "How to use skills")!.click());

    const emitted = await toasts(async () => {
      await act(async () => {
        button(container, "User manual")!.click();
      });
    });

    expect(emitted).toHaveLength(1);
    expect(emitted[0]!.tone).toBe("info");
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
    // No manual configured is not an error: the popover stays for the coach path.
    expect(popover(container)).not.toBeNull();
  });

  it("reports an error toast when the manual lookup fails", async () => {
    mocks.fetchSkillManualUrl.mockRejectedValue(new Error("platform unreachable"));
    const { container } = mount();
    act(() => button(container, "How to use skills")!.click());

    const emitted = await toasts(async () => {
      await act(async () => {
        button(container, "User manual")!.click();
      });
    });

    expect(emitted).toHaveLength(1);
    expect(emitted[0]!.tone).toBe("error");
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("reports an error toast when the browser refuses the link", async () => {
    mocks.openExternalUrl.mockRejectedValue(new Error("no handler"));
    const { container } = mount();
    act(() => button(container, "How to use skills")!.click());

    const emitted = await toasts(async () => {
      await act(async () => {
        button(container, "User manual")!.click();
      });
    });

    // The action was taken (the popover collapsed) but the browser could not
    // open the link, so the user is told rather than left guessing.
    expect(emitted).toHaveLength(1);
    expect(emitted[0]!.tone).toBe("error");
    expect(popover(container)).toBeNull();
    expect(button(container, "How to use skills")!.disabled).toBe(false);
  });

  it("does not start the coach while a manual lookup is in flight", async () => {
    let resolveUrl!: (value: string) => void;
    mocks.fetchSkillManualUrl.mockReturnValue(new Promise((resolve) => {
      resolveUrl = resolve;
    }));
    const { container, onStartCoachConversation } = mount();
    act(() => button(container, "How to use skills")!.click());

    await act(async () => {
      button(container, "User manual")!.click();
    });
    expect(button(container, "User manual")!.disabled).toBe(true);
    // The coach button stays enabled (only the manual action is busy), so the
    // handler's own guard is what has to reject the second action.
    expect(button(container, "Teach me to use skills")!.disabled).toBe(false);

    await act(async () => {
      button(container, "Teach me to use skills")!.click();
    });

    expect(mocks.fetchCoachPrompt).not.toHaveBeenCalled();
    expect(onStartCoachConversation).not.toHaveBeenCalled();

    await act(async () => {
      resolveUrl("https://docs.example.com/manual");
    });
    expect(mocks.openExternalUrl).toHaveBeenCalledTimes(1);
  });
});
