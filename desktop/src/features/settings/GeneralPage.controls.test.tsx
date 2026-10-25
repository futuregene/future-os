// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { setFontSize } from "../../lib/fontSize";
import { DEFAULT_REPLY_DETAIL, getReplyDetail, setReplyDetail } from "../../lib/replyDisplay";
import { setThemeMode, setThemePalette } from "../../lib/theme";
import { GeneralPage } from "./GeneralPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  availability: { available: true, code: null as string | null, resolved: true },
  isLinux: false,
  isWindows: true,
}));

vi.mock("../../integrations/agent/useSandboxAvailability", async importOriginal => ({
  ...await importOriginal<typeof import("../../integrations/agent/useSandboxAvailability")>(),
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
  setThemeMode("system");
  setThemePalette("futureos");
  setFontSize("medium");
  // Back to the shipped level rather than whatever the last case left.
  setReplyDetail(DEFAULT_REPLY_DETAIL);
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
    futureSessionStatus: "authenticated",
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

    // Nothing chosen yet, so the picker sits on "follow the system" (en-US here).
    expect(language.value).toBe("system");
    expect([...language.options].map(option => option.textContent)).toEqual(["Follow system", "中文", "English"]);

    choose(language, "zh");

    expect(i18n.language).toBe("zh");
    expect(document.documentElement.lang).toBe("zh-CN");
    // Stored so the next launch starts in the chosen language.
    expect(localStorage.getItem("future.language")).toBe("zh");
  });

  it("goes back to following the system without pinning a language", () => {
    const { container } = mount();

    choose(select(container, 0), "zh");
    choose(select(container, 0), "system");

    // jsdom's OS locale is en-US, so the system preference resolves to English.
    expect(i18n.language).toBe("en");
    expect(document.documentElement.lang).toBe("en");
    expect(localStorage.getItem("future.language")).toBe("system");
  });

  it("switches back to English and restores the document language", () => {
    const { container } = mount();

    choose(select(container, 0), "zh");
    choose(select(container, 0), "en");

    expect(i18n.language).toBe("en");
    expect(document.documentElement.lang).toBe("en");
  });
});

describe("generalPage appearance picker", () => {
  it("reflects the active theme and switches the document palette", () => {
    const { container } = mount();
    const theme = select(container, 1);

    expect(theme.value).toBe("system");
    expect([...theme.options].map(option => option.textContent)).toEqual(["Light", "Dark", "System"]);

    choose(theme, "dark");

    expect(container.ownerDocument.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("future.theme")).toBe("dark");
    expect(theme.value).toBe("dark");

    choose(theme, "light");

    expect(container.ownerDocument.documentElement.classList.contains("dark")).toBe(false);
    expect(localStorage.getItem("future.theme")).toBe("light");
  });
});

describe("generalPage color theme picker", () => {
  it("offers every palette as a named radio, with the active one checked", () => {
    const { container } = mount();
    const group = container.querySelector("[role=radiogroup][aria-label='Color theme']")!;
    const cards = [...group.querySelectorAll<HTMLButtonElement>("[role=radio]")];

    expect(cards.map(card => card.getAttribute("aria-label")))
      .toEqual(["FutureOS", "Ayu", "Catppuccin", "Dracula", "Everforest", "Gruvbox", "Kanagawa", "Nightfox", "Nord", "One Dark", "Rosé Pine", "Solarized", "Tokyo Night", "Vesper"]);
    expect(cards.map(card => card.getAttribute("aria-checked")))
      .toEqual(cards.map((_, index) => String(index === 0)));
  });

  it("paints each preview in its own palette while the app keeps the active one", () => {
    const { container } = mount();
    const cards = [...container.querySelectorAll<HTMLButtonElement>("[role=radio]")];

    // A card previews a palette by carrying its `data-palette`: the CSS blocks
    // match a nested preview as well as the root.
    expect(cards.map(card => card.dataset.palette))
      .toEqual(["futureos", "ayu", "catppuccin", "dracula", "everforest", "gruvbox", "kanagawa", "nightfox", "nord", "one-dark", "rose-pine", "solarized", "tokyo-night", "vesper"]);
    // ...and the app itself is still on the default.
    expect(document.documentElement.dataset.palette).toBe("futureos");
  });

  it("switches the palette and remembers it", () => {
    const { container } = mount();
    const nord = container.querySelector<HTMLButtonElement>("[role=radio][aria-label='Nord']")!;

    act(() => nord.click());

    expect(document.documentElement.dataset.palette).toBe("nord");
    expect(localStorage.getItem("future.palette")).toBe("nord");
    expect(nord.getAttribute("aria-checked")).toBe("true");
    // The appearance choice is a separate axis and stays where it was.
    expect(localStorage.getItem("future.theme")).toBe("system");
    expect(select(container, 1).value).toBe("system");
  });

  it("leaves the light/dark palette alone when only the color theme changes", () => {
    const { container } = mount();
    const dark = container.querySelector<HTMLButtonElement>("[role=radio][aria-label='Catppuccin']")!;

    act(() => dark.click());

    expect(container.ownerDocument.documentElement.classList.contains("dark")).toBe(false);
    expect(localStorage.getItem("future.palette")).toBe("catppuccin");
  });

  it("restores the stored palette on mount", () => {
    setThemePalette("gruvbox");
    const { container } = mount();
    const checked = container.querySelector("[role=radio][aria-checked='true']")!;

    expect(checked.getAttribute("aria-label")).toBe("Gruvbox");
  });
});

describe("generalPage font size picker", () => {
  it("reflects the active text size and scales the document", () => {
    const { container } = mount();
    const fontSize = select(container, 2);

    expect(fontSize.value).toBe("medium");
    expect([...fontSize.options].map(option => option.textContent)).toEqual(["Small", "Medium", "Large", "Extra large"]);
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("1");

    choose(fontSize, "xlarge");

    expect(fontSize.value).toBe("xlarge");
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("1.35");
    expect(localStorage.getItem("future.fontSize")).toBe("xlarge");

    choose(fontSize, "small");

    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("0.85");
    expect(localStorage.getItem("future.fontSize")).toBe("small");
  });
});

describe("generalPage model output level", () => {
  it("offers default and detailed, opens on default, and remembers the choice", () => {
    const { container } = mount();
    // The fifth picker: language, theme, text size, model output, approval.
    const level = select(container, 3);

    expect(level.value).toBe("default");
    expect([...level.options].map(option => option.value)).toEqual(["default", "detailed"]);
    expect([...level.options].map(option => option.textContent)).toEqual(["Default", "Detailed"]);
    expect(container.textContent).toContain("shows its final answer");

    choose(level, "detailed");

    expect(getReplyDetail()).toBe("detailed");
    expect(localStorage.getItem("future.reply.detail")).toBe("detailed");
    // The row's own words follow the level it is on.
    expect(container.textContent).toContain("keeps everything");

    choose(level, "default");
    expect(getReplyDetail()).toBe("default");
    expect(localStorage.getItem("future.reply.detail")).toBe("default");
  });
});

describe("generalPage approval tier picker", () => {
  it("reports every tier change to the settings owner", () => {
    const onChangeApprovalTier = vi.fn();
    const { container } = mount({ approvalTier: "off", onChangeApprovalTier });
    const tier = select(container, 4);

    expect(tier.value).toBe("off");
    expect([...tier.options].map(option => option.value)).toEqual(["manual", "sandbox", "auto", "off"]);

    choose(tier, "manual");
    expect(onChangeApprovalTier).toHaveBeenLastCalledWith("manual");
    choose(tier, "auto");
    expect(onChangeApprovalTier).toHaveBeenLastCalledWith("auto");
  });

  it.each(["signed_out", "invalid", "checking"])("gates automatic review for %s without changing a pending preference", (futureSessionStatus) => {
    const { container } = mount({ approvalTier: "auto", futureSessionStatus });
    const tier = select(container, 4);
    expect(tier.value).toBe(futureSessionStatus === "checking" ? "auto" : "sandbox");
    expect([...tier.options].find(option => option.value === "auto")!.disabled).toBe(true);
    expect([...tier.options].find(option => option.value === "sandbox")!.disabled).toBe(false);
  });

  it("enables the sandbox option when the sandbox is available", () => {
    const { container } = mount({ approvalTier: "sandbox" });
    const sandbox = [...select(container, 4).options].find(option => option.value === "sandbox")!;

    expect(sandbox.disabled).toBe(false);
    expect(sandbox.textContent).toBe("Sandboxed");
  });

  it("disables the sandbox option and explains it while the probe is unresolved", () => {
    mocks.availability = { available: false, code: null, resolved: false };
    const { container } = mount();

    const sandbox = [...select(container, 4).options].find(option => option.value === "sandbox")!;
    expect(sandbox.disabled).toBe(true);
    expect(sandbox.textContent).toBe("Sandboxed (checking)");
  });

  it("labels an unavailable sandbox", () => {
    mocks.availability = { available: false, code: "probe_failed", resolved: true };
    const { container } = mount();

    const sandbox = [...select(container, 4).options].find(option => option.value === "sandbox")!;
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
    ["version_too_old", "This Bubblewrap package cannot provide the required protection"],
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

    const toggle = (label: string) => container.querySelector<HTMLButtonElement>(`[role=switch][aria-label='${label}']`)!;

    act(() => toggle("Auto-upgrade skills").click());
    act(() => toggle("Completion bell").click());

    expect(props.onToggleAutoUpgradeSkills).toHaveBeenCalledExactlyOnceWith(false);
    expect(props.onToggleBellOnComplete).toHaveBeenCalledExactlyOnceWith(false);
  });
});
