// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { GeneralPage } from "./GeneralPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  availability: { available: true, code: null as string | null, resolved: true },
  isLinux: false,
  isWindows: true,
}));

vi.mock("../../integrations/agent/useSandboxAvailability", () => ({
  useSandboxAvailability: () => mocks.availability,
}));
vi.mock("../../lib/platform", () => ({
  get isLinux() {
    return mocks.isLinux;
  },
  get isWindows() {
    return mocks.isWindows;
  },
}));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.availability = { available: true, code: null, resolved: true };
  mocks.isLinux = false;
  mocks.isWindows = true;
  void i18n.changeLanguage("en");
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
  void i18n.changeLanguage("en");
});

function mount(overrides: Partial<React.ComponentProps<typeof GeneralPage>> = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  const props = {
    approvalTier: "off" as const,
    onChangeApprovalTier: vi.fn(),
    autoUpgradeSkills: false,
    onToggleAutoUpgradeSkills: vi.fn(),
    skillRecommend: true,
    onToggleSkillRecommend: vi.fn(),
    bellOnComplete: true,
    onToggleBellOnComplete: vi.fn(),
    autoTitleFirstTurn: true,
    onToggleAutoTitleFirstTurn: vi.fn(),
    ...overrides,
  };
  act(() => {
    root.render(<GeneralPage {...props} />);
  });
  return { container, props };
}

function select(container: HTMLElement, index: number) {
  return container.querySelectorAll<HTMLSelectElement>("select")[index]!;
}

function choose(input: HTMLSelectElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

describe("generalPage language picker", () => {
  it("reflects the active language and switches it", () => {
    const { container } = mount();
    const language = select(container, 0);

    expect(language.value).toBe("en");
    expect([...language.options].map(option => option.textContent)).toEqual(["中文", "English"]);

    choose(language, "zh");

    expect(i18n.language).toBe("zh");
    expect(document.documentElement.lang).toBe("zh-CN");
    // Stored so the next launch starts in the chosen language.
    expect(localStorage.getItem("future.language")).toBe("zh");
  });

  it("switches back to English and restores the document language", () => {
    const { container } = mount();

    choose(select(container, 0), "zh");
    choose(select(container, 0), "en");

    expect(i18n.language).toBe("en");
    expect(document.documentElement.lang).toBe("en");
  });
});

describe("generalPage approval tier picker", () => {
  it("reports every tier change to the settings owner", () => {
    const onChangeApprovalTier = vi.fn();
    const { container } = mount({ approvalTier: "off", onChangeApprovalTier });
    const tier = select(container, 1);

    expect(tier.value).toBe("off");
    expect([...tier.options].map(option => option.value)).toEqual(["manual", "sandbox", "off"]);

    choose(tier, "manual");
    expect(onChangeApprovalTier).toHaveBeenLastCalledWith("manual");
  });

  it("enables the sandbox option when the sandbox is available", () => {
    const { container } = mount({ approvalTier: "sandbox" });
    const sandbox = [...select(container, 1).options].find(option => option.value === "sandbox")!;

    expect(sandbox.disabled).toBe(false);
    expect(sandbox.textContent).toBe("Sandboxed");
  });

  it("disables the sandbox option and explains it while the probe is unresolved", () => {
    mocks.availability = { available: false, code: null, resolved: false };
    const { container } = mount();

    const sandbox = [...select(container, 1).options].find(option => option.value === "sandbox")!;
    expect(sandbox.disabled).toBe(true);
    expect(sandbox.textContent).toBe("Sandboxed (checking)");
  });

  it("labels an unavailable sandbox", () => {
    mocks.availability = { available: false, code: "probe_failed", resolved: true };
    const { container } = mount();

    const sandbox = [...select(container, 1).options].find(option => option.value === "sandbox")!;
    expect(sandbox.disabled).toBe(true);
    expect(sandbox.textContent).toBe("Sandboxed (unavailable)");
  });

  it.each([
    ["manual", "File access follows approval rules"],
    ["off", "No prompts, no sandbox"],
  ] as const)("describes the %s tier", (approvalTier, expected) => {
    const { container } = mount({ approvalTier });

    expect(container.textContent).toContain(expected);
  });

  it("uses the Windows wording for the sandbox tier on Windows", () => {
    mocks.isWindows = true;
    mocks.isLinux = false;
    const { container } = mount({ approvalTier: "sandbox" });

    expect(container.textContent).toContain("Commands run with Windows write protection");
  });

  it("uses the Linux wording for the sandbox tier on Linux", () => {
    mocks.isWindows = false;
    mocks.isLinux = true;
    const { container } = mount({ approvalTier: "sandbox" });

    expect(container.textContent).toContain("Commands run in the Linux sandbox");
  });

  it("falls back to the generic sandbox wording on macOS", () => {
    mocks.isWindows = false;
    mocks.isLinux = false;
    const { container } = mount({ approvalTier: "sandbox" });

    expect(container.textContent).toContain("Commands run in the macOS sandbox");
  });

  it("explains an unavailable Linux sandbox with the probe code", () => {
    mocks.isWindows = false;
    mocks.isLinux = true;
    mocks.availability = { available: false, code: "binary_missing", resolved: true };
    const { container } = mount();

    expect(container.textContent).toContain("Linux sandbox unavailable");
    expect(container.textContent).toContain("Bubblewrap is not installed");
    expect(container.textContent).toContain("(binary_missing)");
  });

  it.each([
    ["path_rejected", "No trusted system Bubblewrap was found"],
    ["version_too_old", "Bubblewrap is older than version 0.9.0"],
    ["user_namespace_disabled", "Unprivileged user namespaces are disabled"],
    ["probe_transport_error", "could not contact the Agent"],
  ] as const)("maps the %s diagnostic to its remediation copy", (code, expected) => {
    mocks.isWindows = false;
    mocks.isLinux = true;
    mocks.availability = { available: false, code, resolved: true };
    const { container } = mount();

    expect(container.textContent).toContain(expected);
  });

  it("uses the generic probe-failed code when the probe reported none", () => {
    mocks.isWindows = false;
    mocks.isLinux = true;
    mocks.availability = { available: false, code: null, resolved: true };
    const { container } = mount();

    expect(container.textContent).toContain("Linux sandbox unavailable");
    expect(container.textContent).toContain("probe_failed");
  });

  it("hides the Linux guidance once the probe reports the sandbox available", () => {
    mocks.isWindows = false;
    mocks.isLinux = true;
    mocks.availability = { available: true, code: null, resolved: true };
    const { container } = mount();

    expect(container.textContent).not.toContain("Linux sandbox unavailable");
  });

  it("does not show the Linux guidance on Windows", () => {
    mocks.isWindows = true;
    mocks.isLinux = false;
    mocks.availability = { available: false, code: "binaryMissing", resolved: true };
    const { container } = mount();

    expect(container.textContent).not.toContain("Linux sandbox unavailable");
  });
});

describe("generalPage switches", () => {
  it("exposes every toggle as an accessible switch with its current value", () => {
    const { container } = mount({ autoUpgradeSkills: true, skillRecommend: false, bellOnComplete: false, autoTitleFirstTurn: true });

    const switches = [...container.querySelectorAll<HTMLButtonElement>("[role=switch]")];
    expect(switches.map(item => item.getAttribute("aria-label"))).toEqual([
      "Auto-upgrade skills",
      "Skill recommendations",
      "Generate a title after the first answer",
      "Completion bell",
    ]);
    expect(switches.map(item => item.getAttribute("aria-checked"))).toEqual(["true", "false", "true", "false"]);
  });

  it("reports each toggle to its own callback, inverted", () => {
    const { container, props } = mount({ autoUpgradeSkills: true, bellOnComplete: true });

    act(() => container.querySelectorAll<HTMLButtonElement>("[role=switch]")[0]!.click());
    act(() => container.querySelectorAll<HTMLButtonElement>("[role=switch]")[3]!.click());

    expect(props.onToggleAutoUpgradeSkills).toHaveBeenCalledExactlyOnceWith(false);
    expect(props.onToggleBellOnComplete).toHaveBeenCalledExactlyOnceWith(false);
  });
});
