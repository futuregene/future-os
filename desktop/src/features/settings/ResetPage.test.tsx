// @vitest-environment jsdom
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommunityEditionSection } from "./CommunityEditionSection";
import { ResetPage } from "./ResetPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ invokeCommand: vi.fn(), isWindows: true }));

vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));
// `isWindows` is a module-level constant derived from the user agent, so it is
// mocked as a getter to exercise both platform branches in one suite.
vi.mock("../../lib/platform", () => ({
  get isWindows() {
    return mocks.isWindows;
  },
}));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.invokeCommand.mockReset();
  mocks.isWindows = true;
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => root.render(node));
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement | undefined;
}

describe("resetPage write protection (Windows)", () => {
  it("resets the Windows sandbox and reports success", async () => {
    let resolveReset!: (value: number) => void;
    mocks.invokeCommand.mockReturnValue(new Promise<number>((resolve) => {
      resolveReset = resolve;
    }));
    const container = mount(<ResetPage />);
    expect(container.textContent).toContain("Write-protection permissions");

    await act(async () => {
      button(container, "Reset write protection")!.click();
    });
    expect(mocks.invokeCommand).toHaveBeenCalledWith("reset_windows_sandbox");
    expect(button(container, "Resetting…")!.disabled).toBe(true);

    await act(async () => {
      resolveReset(0);
    });
    expect(container.textContent).toContain("Write-protection permissions were reset.");
    expect(button(container, "Reset write protection")!.disabled).toBe(false);
  });

  it("reports a failure without leaking backend diagnostics", async () => {
    mocks.invokeCommand.mockRejectedValue(new Error("SID S-1-5-21 denied at C:\\state.json"));
    const container = mount(<ResetPage />);

    await act(async () => {
      button(container, "Reset write protection")!.click();
    });

    expect(container.textContent).toContain("Reset is not available right now.");
    expect(container.textContent).not.toContain("S-1-5-21");
    expect(button(container, "Reset write protection")!.disabled).toBe(false);
  });

  it("hides the write-protection section on other platforms", () => {
    mocks.isWindows = false;
    const container = mount(<ResetPage />);

    expect(container.textContent).not.toContain("Write-protection permissions");
    expect(container.textContent).toContain("Clear data");
  });
});

describe("resetPage clear data", () => {
  it("asks twice, and cancel returns to the plain action", () => {
    const container = mount(<ResetPage />);

    act(() => button(container, "Clear data")!.click());
    expect(container.textContent).toContain("Clear all local data and restart?");
    expect(mocks.invokeCommand).not.toHaveBeenCalled();

    act(() => button(container, "Cancel")!.click());
    expect(container.textContent).not.toContain("Clear all local data and restart?");
    expect(button(container, "Clear data")).toBeTruthy();
  });

  it("stays busy after a successful wipe (the backend restarts the app)", async () => {
    mocks.invokeCommand.mockResolvedValue(undefined);
    const container = mount(<ResetPage />);

    act(() => button(container, "Clear data")!.click());
    await act(async () => {
      button(container, "Confirm clear")!.click();
    });

    expect(mocks.invokeCommand).toHaveBeenCalledWith("clear_app_data");
    expect(button(container, "Clearing…")!.disabled).toBe(true);
  });

  it("reports an Error wipe failure and re-arms the action", async () => {
    mocks.invokeCommand.mockRejectedValue(new Error("database is locked"));
    const container = mount(<ResetPage />);

    act(() => button(container, "Clear data")!.click());
    await act(async () => {
      button(container, "Confirm clear")!.click();
    });

    expect(container.textContent).toContain("database is locked");
    expect(container.textContent).not.toContain("Clear all local data and restart?");
    expect(button(container, "Clear data")).toBeTruthy();
  });

  it("stringifies a non-Error wipe failure", async () => {
    mocks.invokeCommand.mockRejectedValue("storage offline");
    const container = mount(<ResetPage />);

    act(() => button(container, "Clear data")!.click());
    await act(async () => {
      button(container, "Confirm clear")!.click();
    });

    expect(container.textContent).toContain("storage offline");
  });
});

describe("communityEditionSection", () => {
  it("only saves a changed selection", async () => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    const container = mount(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={onChange} />);
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch]")!;

    expect(button(container, "Switch")!.disabled).toBe(true);
    expect(toggle.getAttribute("aria-checked")).toBe("false");

    act(() => toggle.click());
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(button(container, "Switch")!.disabled).toBe(false);

    await act(async () => {
      button(container, "Switch")!.click();
    });

    expect(onChange).toHaveBeenCalledExactlyOnceWith(true);
    expect(button(container, "Switch")!.disabled).toBe(false);
  });

  it("ignores a save request that changes nothing, even if the click lands", () => {
    const onChange = vi.fn().mockResolvedValue(undefined);
    const container = mount(<CommunityEditionSection communityEdition onChangeCommunityEdition={onChange} />);
    const save = button(container, "Switch")!;

    expect(save.disabled).toBe(true);
    // Fault injection: a delivered activation of the unchanged control (an
    // atypical UA / assistive path that still dispatches the click). React's
    // own `disabled` gate is dropped on purpose so the handler's guard is what
    // has to reject it — and it must, or the settings get rewritten for
    // nothing.
    save.removeAttribute("disabled");
    act(() => save.click());

    expect(onChange).not.toHaveBeenCalled();
    expect(container.querySelector("[role=alert]")).toBeNull();
    expect(container.querySelector<HTMLButtonElement>("[role=switch]")!.getAttribute("aria-checked")).toBe("true");
  });

  it("keeps saving state across an in-flight save", async () => {
    let resolveSave!: () => void;
    const onChange = vi.fn(() => new Promise<void>((resolve) => {
      resolveSave = resolve;
    }));
    const container = mount(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={onChange} />);
    act(() => container.querySelector<HTMLButtonElement>("[role=switch]")!.click());

    act(() => button(container, "Switch")!.click());
    expect(button(container, "Switching…")!.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>("[role=switch]")!.disabled).toBe(true);

    await act(async () => {
      resolveSave();
    });
    expect(button(container, "Switch")).toBeTruthy();
  });

  it("surfaces a save failure and remains switchable", async () => {
    const onChange = vi.fn().mockRejectedValue(new Error("settings file is read-only"));
    const container = mount(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={onChange} />);
    act(() => container.querySelector<HTMLButtonElement>("[role=switch]")!.click());

    await act(async () => {
      button(container, "Switch")!.click();
    });

    expect(container.querySelector("[role=alert]")!.textContent).toBe("settings file is read-only");
    expect(button(container, "Switch")!.disabled).toBe(false);
  });

  it("stringifies a non-Error save failure and clears it on retry", async () => {
    const onChange = vi.fn().mockRejectedValueOnce("disk full").mockResolvedValueOnce(undefined);
    const container = mount(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={onChange} />);
    act(() => container.querySelector<HTMLButtonElement>("[role=switch]")!.click());

    await act(async () => {
      button(container, "Switch")!.click();
    });
    expect(container.querySelector("[role=alert]")!.textContent).toBe("disk full");

    await act(async () => {
      button(container, "Switch")!.click();
    });
    expect(container.querySelector("[role=alert]")).toBeNull();
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("adopts a value confirmed elsewhere", () => {
    const container = mount(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={vi.fn()} />);
    act(() => roots[0]!.root.render(
      <CommunityEditionSection communityEdition onChangeCommunityEdition={vi.fn()} />,
    ));

    expect(container.querySelector<HTMLButtonElement>("[role=switch]")!.getAttribute("aria-checked")).toBe("true");
    expect(button(container, "Switch")!.disabled).toBe(true);
  });
});
