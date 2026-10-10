// @vitest-environment jsdom
import type { FutureEnvironment } from "../../integrations/agent/providers";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EnvironmentPage } from "./EnvironmentPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ getFutureEnvironment: vi.fn(), invokeCommand: vi.fn() }));

vi.mock("../../integrations/agent/providers", () => ({ getFutureEnvironment: mocks.getFutureEnvironment }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.getFutureEnvironment.mockReset();
  mocks.invokeCommand.mockReset();
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

async function renderPage() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(<EnvironmentPage />);
  });
  return container;
}

function select(container: HTMLElement) {
  return container.querySelector<HTMLSelectElement>("select")!;
}

function switchButton(container: HTMLElement) {
  return [...container.querySelectorAll("button")].find(
    item => item.textContent === "Switch and restart" || item.textContent === "Switching…",
  ) as HTMLButtonElement | undefined;
}

function choose(container: HTMLElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select(container), value);
    select(container).dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("environmentPage", () => {
  it("defaults to the active environment and refuses to switch to it", async () => {
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "production", platformUrl: "https://prod.example.com" });
    const container = await renderPage();

    expect(select(container).value).toBe("production");
    expect(container.textContent).toContain("Current: https://prod.example.com");
    expect(switchButton(container)!.disabled).toBe(true);
  });

  it("switches to the other environment and stays busy (the backend restarts the app)", async () => {
    const target: FutureEnvironment = { environment: "production", platformUrl: "https://prod.example.com" };
    mocks.getFutureEnvironment.mockResolvedValue(target);
    mocks.invokeCommand.mockResolvedValue(undefined);
    const container = await renderPage();

    choose(container, "test");
    expect(select(container).value).toBe("test");
    expect(switchButton(container)!.disabled).toBe(false);
    // The picker only knows ids; the URL stays on the page as the *active* one.
    expect(container.textContent).toContain("Current: https://prod.example.com");

    await act(async () => {
      switchButton(container)!.click();
    });

    expect(mocks.invokeCommand).toHaveBeenCalledWith("set_future_environment", { environment: "test" });
    expect(switchButton(container)!.disabled).toBe(true);
  });

  it("ignores a switch request while the picker matches the active environment", async () => {
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "production", platformUrl: "https://prod.example.com" });
    const container = await renderPage();
    const save = switchButton(container)!;

    expect(save.disabled).toBe(true);
    // Fault injection: drop React's `disabled` gate so the click is delivered and
    // the handler's own `if (!changed) return` has to reject the no-op restart.
    save.removeAttribute("disabled");
    await act(async () => {
      save.click();
    });

    expect(mocks.invokeCommand).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Switching…");
  });

  it("reports a failed switch and re-enables the button", async () => {
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "production", platformUrl: "https://prod.example.com" });
    mocks.invokeCommand.mockRejectedValue(new Error("restart refused"));
    const container = await renderPage();

    choose(container, "test");
    await act(async () => {
      switchButton(container)!.click();
    });

    expect(container.textContent).toContain("restart refused");
    expect(switchButton(container)!.disabled).toBe(false);
  });

  it("stringifies a non-Error switch failure", async () => {
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "test", platformUrl: "https://test.example.com" });
    mocks.invokeCommand.mockRejectedValue("no such environment");
    const container = await renderPage();

    choose(container, "production");
    await act(async () => {
      switchButton(container)!.click();
    });

    expect(container.textContent).toContain("no such environment");
  });

  it("shows a custom environment as unknown and keeps switching disabled", async () => {
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "custom", platformUrl: "https://self.example.com" });
    const container = await renderPage();

    const options = [...select(container).options].map(option => option.textContent);
    expect(options).toContain("Custom environment");
    expect(select(container).value).toBe("");
    expect(container.textContent).toContain("Current: https://self.example.com");
    expect(switchButton(container)!.disabled).toBe(true);
  });

  it("shows the loading placeholder while the environment is still unknown", async () => {
    let resolveEnv!: (value: FutureEnvironment) => void;
    mocks.getFutureEnvironment.mockReturnValue(new Promise((resolve) => {
      resolveEnv = resolve;
    }));
    const container = await renderPage();

    expect(select(container).disabled).toBe(true);
    expect(container.textContent).toContain("Loading…");

    await act(async () => {
      resolveEnv({ environment: "production", platformUrl: "https://prod.example.com" });
    });
    expect(select(container).disabled).toBe(false);
    expect(select(container).value).toBe("production");
  });

  it("reports a failed environment lookup as Unknown", async () => {
    mocks.getFutureEnvironment.mockRejectedValue(new Error("agent offline"));
    const container = await renderPage();

    expect(container.textContent).toContain("agent offline");
    expect(container.textContent).toContain("Current: Unknown");
    expect(select(container).value).toBe("");
  });
});
