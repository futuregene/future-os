// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SkillGuideBanner } from "./SkillGuideBanner";
import { SkillIntroBubble } from "./SkillIntroBubble";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(node: React.ReactElement) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => {
    root.render(node);
  });
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.includes(text)) as HTMLButtonElement | undefined;
}

describe("skillGuideBanner", () => {
  it("explains the coach conversation and starts it on demand", () => {
    const onStart = vi.fn();
    const container = mount(<SkillGuideBanner starting={false} onStart={onStart} onDismiss={vi.fn()} />);

    expect(container.textContent).toContain("5-Minute Skill Onboarding");
    expect(container.textContent).toContain("Interactive coaching");
    expect(container.querySelector("svg")).not.toBeNull();

    act(() => button(container, "Skill Tutorial")!.click());
    expect(onStart).toHaveBeenCalledTimes(1);
  });

  it("locks the start action while the conversation is being created", () => {
    const onStart = vi.fn();
    const container = mount(<SkillGuideBanner starting onStart={onStart} onDismiss={vi.fn()} />);

    const start = button(container, "Skill Tutorial")!;
    expect(start.disabled).toBe(true);

    // Fault injection: a delivered activation of the in-flight button must still
    // be a no-op, so one click cannot create two coach conversations.
    start.removeAttribute("disabled");
    act(() => start.click());
    expect(onStart).not.toHaveBeenCalled();
  });

  it("dismisses from the corner control, which is named for assistive tech", () => {
    const onDismiss = vi.fn();
    const container = mount(<SkillGuideBanner starting={false} onStart={vi.fn()} onDismiss={onDismiss} />);

    const close = container.querySelector<HTMLButtonElement>("button[aria-label='Dismiss tutorial entry']")!;
    expect(close).toBeTruthy();

    act(() => close.click());
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});

describe("skillIntroBubble", () => {
  it("points at the nav entry with the installed count and both actions", () => {
    const onGo = vi.fn();
    const onDismiss = vi.fn();
    const container = mount(<SkillIntroBubble count={3} onGo={onGo} onDismiss={onDismiss} />);

    expect(container.textContent).toContain("3 common skills installed for you");
    expect(container.textContent).toContain("Take a look");

    act(() => button(container, "Take a look")!.click());
    expect(onGo).toHaveBeenCalledTimes(1);

    act(() => button(container, "Got it")!.click());
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onGo).toHaveBeenCalledTimes(1);
  });

  it("reads correctly with nothing installed yet", () => {
    const container = mount(<SkillIntroBubble count={0} onGo={vi.fn()} onDismiss={vi.fn()} />);

    expect(container.textContent).toContain("No skills installed yet");
    expect(container.textContent).toContain("Got it");
  });

  it("stays dismissed-explicit: no outside-click handler is installed", () => {
    const onDismiss = vi.fn();
    const container = mount(<SkillIntroBubble count={1} onGo={vi.fn()} onDismiss={onDismiss} />);

    act(() => {
      document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      document.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    });

    expect(onDismiss).not.toHaveBeenCalled();
    expect(container.textContent).toContain("1 common skills installed for you");
  });
});
