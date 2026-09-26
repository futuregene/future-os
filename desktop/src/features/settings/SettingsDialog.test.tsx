// @vitest-environment jsdom
import type { UpdateStatus } from "../../components/layout/hooks/useUpdateChecker";
import type { AppSettings } from "../../integrations/storage/appSettings";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS } from "../../integrations/storage/appSettings";
import { SettingsDialog } from "./SettingsDialog";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ buildInfo: null as { version: string; isRelease: boolean } | null }));

vi.mock("../../integrations/tauri/useBuildInfo", () => ({
  useBuildInfo: () => ({ data: mocks.buildInfo }),
}));

// Each page has its own suite and its own IPC surface. Here the dialog itself is
// under test: every page is a probe that reports the props the dialog handed it,
// so routing, the dev-only gate and the settings-patch mapping are what fail if
// they regress.
vi.mock("./GeneralPage", () => ({
  GeneralPage: (props: Record<string, unknown>) => (
    <div
      data-page="general"
      data-approval={String(props.approvalTier)}
      data-auto-title={String(props.autoTitleFirstTurn)}
      data-bell={String(props.bellOnComplete)}
      data-auto-upgrade={String(props.autoUpgradeSkills)}
      data-skill-recommend={String(props.skillRecommend)}
    >
      <button data-approval-manual onClick={() => (props.onChangeApprovalTier as (v: string) => void)("manual")}>approval</button>
      <button data-toggle-title onClick={() => (props.onToggleAutoTitleFirstTurn as (v: boolean) => void)(false)}>title</button>
      <button data-toggle-bell onClick={() => (props.onToggleBellOnComplete as (v: boolean) => void)(false)}>bell</button>
      <button data-toggle-upgrade onClick={() => (props.onToggleAutoUpgradeSkills as (v: boolean) => void)(true)}>upgrade</button>
      <button data-toggle-recommend onClick={() => (props.onToggleSkillRecommend as (v: boolean) => void)(false)}>recommend</button>
    </div>
  ),
}));
vi.mock("./RemotePage", () => ({
  RemotePage: (props: { autoConnectRemote: boolean; onToggleAutoConnectRemote: (v: boolean) => void }) => (
    <div data-page="remote" data-auto-connect={String(props.autoConnectRemote)}>
      <button data-toggle-auto-connect onClick={() => props.onToggleAutoConnectRemote(true)}>auto-connect</button>
    </div>
  ),
}));
vi.mock("./AccountPage", () => ({
  AccountPage: (props: Record<string, unknown>) => (
    <div
      data-page="account"
      data-balance={String(props.balance)}
      data-balance-status={String(props.balanceStatus)}
      data-email={String(props.email)}
      data-session={String(props.sessionStatus)}
      data-community={String(props.communityEdition)}
    >
      <button data-refresh-auth onClick={() => (props.onRefreshAuth as () => void)()} />
      <button data-refresh-balance onClick={() => (props.onRefreshBalance as () => void)()} />
    </div>
  ),
}));
vi.mock("./UpdatePage", () => ({
  UpdatePage: (props: { cachedStatus: unknown }) => <div data-page="update" data-cached={String(props.cachedStatus !== null)} />,
}));
vi.mock("./AboutPage", () => ({ AboutPage: () => <div data-page="about" /> }));
vi.mock("./ProvidersPage", () => ({
  ProvidersPage: (props: Record<string, unknown>) => (
    <div
      data-page="providers"
      data-community={String(props.communityEdition)}
      data-session={String(props.futureSessionStatus)}
    >
      <button data-refresh-providers-auth onClick={() => (props.onRefreshFutureAuth as () => void)()} />
      <button data-providers-changed onClick={() => (props.onProvidersChanged as (() => void) | undefined)?.()} />
    </div>
  ),
}));
vi.mock("./ModelsPage", () => ({
  ModelsPage: (props: { hiddenModels: string[]; modelOptions: unknown[]; onChangeHidden: (v: string[]) => void }) => (
    <div data-page="models" data-hidden={props.hiddenModels.join(",")} data-options={String(props.modelOptions.length)}>
      <button data-hide-model onClick={() => props.onChangeHidden(["future/gpt-5"])}>hide</button>
    </div>
  ),
}));
vi.mock("./EnvironmentPage", () => ({ EnvironmentPage: () => <div data-page="environment" /> }));
vi.mock("./CommunityEditionSection", () => ({
  CommunityEditionSection: (props: { communityEdition: boolean; onChangeCommunityEdition: (v: boolean) => void }) => (
    <div data-page="community-edition" data-community={String(props.communityEdition)}>
      <button data-community-switch onClick={() => props.onChangeCommunityEdition(true)}>switch</button>
    </div>
  ),
}));
vi.mock("./ResetPage", () => ({ ResetPage: () => <div data-page="reset" /> }));

const APP_SETTINGS: AppSettings = {
  ...DEFAULT_APP_SETTINGS,
  approvalTier: "manual",
  autoUpgradeSkills: true,
  autoConnectRemote: true,
  bellOnComplete: false,
  autoTitleFirstTurn: false,
  communityEdition: true,
  hiddenModels: ["deepseek/deepseek-chat"],
  skillRecommend: false,
};

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.buildInfo = { version: "1.0.0-dev.abc", isRelease: false };
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

interface DialogOverrides {
  appSettings?: Partial<AppSettings>;
  cachedUpdateStatus?: UpdateStatus | null;
  futureBalance?: number | null;
  futureSessionStatus?: "signed_out" | "authenticated" | "invalid" | "unavailable" | "checking";
  hasUpdate?: boolean;
  initialTab?: string;
  modelOptions?: unknown[];
  onChangeSettings?: (patch: Record<string, unknown>) => Promise<void>;
  onClose?: () => void;
  onProvidersChanged?: () => void;
  onRefreshFutureAuth?: () => void;
  onRefreshFutureBalance?: () => void;
  onUpdateSeen?: () => void;
  open?: boolean;
}

function mount(overrides: DialogOverrides = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  const current: DialogOverrides = { ...overrides };
  function render(next: Partial<DialogOverrides> = {}) {
    Object.assign(current, next);
    act(() => {
      root.render(
        <SettingsDialog
          appSettings={{ ...APP_SETTINGS, ...current.appSettings }}
          cachedUpdateStatus={current.cachedUpdateStatus ?? null}
          futureBalance={current.futureBalance ?? null}
          futureBalanceStatus="available"
          futureEmail="user@example.com"
          futureSessionStatus={current.futureSessionStatus ?? "authenticated"}
          hasUpdate={current.hasUpdate}
          initialTab={current.initialTab as never}
          modelOptions={(current.modelOptions ?? [{ id: "m" }]) as never}
          onChangeSettings={current.onChangeSettings ?? vi.fn().mockResolvedValue(undefined)}
          onClose={current.onClose ?? vi.fn()}
          onProvidersChanged={current.onProvidersChanged}
          onRefreshFutureAuth={current.onRefreshFutureAuth ?? vi.fn()}
          onRefreshFutureBalance={current.onRefreshFutureBalance ?? vi.fn()}
          onUpdateSeen={current.onUpdateSeen}
          open={current.open ?? true}
        />,
      );
    });
  }
  render();
  return { container, render };
}

function navButton(container: HTMLElement, label: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === label) as HTMLButtonElement;
}

function navLabels(container: HTMLElement) {
  return [...container.querySelectorAll("nav button")].map(item => item.textContent);
}

function page(container: HTMLElement) {
  return container.querySelector<HTMLElement>("[data-page]")?.dataset.page ?? null;
}

function header(container: HTMLElement) {
  return container.querySelector("header h2")!.textContent;
}

describe("settingsDialog shell", () => {
  it("renders nothing while closed", () => {
    const { container } = mount({ open: false });

    expect(container.textContent).toBe("");
    expect(page(container)).toBeNull();
  });

  it("is a modal dialog labelled for assistive tech", () => {
    const { container } = mount();

    const dialog = container.querySelector("[role=dialog]")!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.getAttribute("aria-label")).toBe("Settings");
  });

  it("groups the navigation into desktop / server / debug sections", () => {
    const { container } = mount();

    const labels = container.querySelectorAll("nav [class*=uppercase]");
    expect([...labels].map(node => node.textContent)).toEqual(["Desktop", "Server", "Debug"]);
  });

  it("closes on Escape and on the backdrop", () => {
    const onClose = vi.fn();
    const { container } = mount({ onClose });

    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => container.querySelector<HTMLButtonElement>("button[aria-label=Close]")!.click());
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("shows the test-build hint on a dev build and nothing on a release build", () => {
    const dev = mount();
    expect(dev.container.textContent).toContain("Test build");

    mocks.buildInfo = { version: "1.0.0", isRelease: true };
    const release = mount();
    expect(release.container.textContent).not.toContain("Test build");
  });
});

describe("settingsDialog tab routing", () => {
  it("opens on General by default", () => {
    const { container } = mount();

    expect(page(container)).toBe("general");
    expect(header(container)).toBe("General");
  });

  it.each([
    ["Phone Control", "remote", "Phone Control"],
    ["Account", "account", "Account"],
    ["Check for updates", "update", "Check for updates"],
    ["About", "about", "About"],
    ["Providers", "providers", "Providers"],
    ["Models", "models", "Models"],
    ["Environment", "environment", "Environment"],
    ["Reset", "reset", "Reset"],
  ] as const)("switches to %s", (nav, expectedPage, expectedHeader) => {
    const { container } = mount();

    act(() => navButton(container, nav).click());

    expect(page(container)).toBe(expectedPage);
    expect(header(container)).toBe(expectedHeader);
  });

  it("marks the active tab and moves the marker with the selection", () => {
    const { container } = mount();

    expect(navButton(container, "General").className).toContain("shadow-xs");
    expect(navButton(container, "Reset").className).not.toContain("shadow-xs");

    act(() => navButton(container, "Reset").click());

    expect(navButton(container, "Reset").className).toContain("shadow-xs");
    expect(navButton(container, "General").className).not.toContain("shadow-xs");
  });

  it("honours initialTab and resets to it on every reopen", () => {
    const { container, render } = mount({ initialTab: "models" });
    expect(page(container)).toBe("models");

    act(() => navButton(container, "About").click());
    expect(page(container)).toBe("about");

    // Close, then reopen with a different requested tab.
    render({ open: false });
    expect(page(container)).toBeNull();
    render({ open: true, initialTab: "reset" });
    expect(page(container)).toBe("reset");
  });

  it("keeps the chosen tab while the dialog stays open", () => {
    const { container, render } = mount({ initialTab: "about" });

    act(() => navButton(container, "Account").click());
    // A re-render with the same props must not snap back to initialTab.
    render({});
    expect(page(container)).toBe("account");
  });
});

describe("settingsDialog dev-only environment tab", () => {
  it("hides the environment tab on a release build", () => {
    mocks.buildInfo = { version: "1.0.0", isRelease: true };
    const { container } = mount();

    expect(navLabels(container)).not.toContain("Environment");
    expect(navLabels(container)).toEqual(["General", "Phone Control", "Account", "Check for updates", "About", "Providers", "Models", "Reset"]);
  });

  it("shows the environment tab (and its community-edition section) on a dev build", () => {
    const { container } = mount();

    expect(navLabels(container)).toContain("Environment");

    act(() => navButton(container, "Environment").click());
    expect(container.querySelector("[data-page=environment]")).not.toBeNull();
    expect(container.querySelector("[data-page=community-edition]")).not.toBeNull();
    // No other tab body leaks into the environment view.
    expect(container.querySelectorAll("[data-page]")).toHaveLength(2);
  });

  it("falls back to General when the build turns out to be a release while the environment tab is open", () => {
    const { container, render } = mount();

    act(() => navButton(container, "Environment").click());
    expect(page(container)).toBe("environment");

    // Build info arrives/updates after the user opened the dev-only tab.
    mocks.buildInfo = { version: "1.0.0", isRelease: true };
    render({});

    expect(page(container)).toBe("general");
    expect(navLabels(container)).not.toContain("Environment");
  });

  it("hides the environment tab until the build info resolves", () => {
    mocks.buildInfo = null;
    const { container } = mount();

    expect(navLabels(container)).not.toContain("Environment");
    expect(container.textContent).not.toContain("Test build");
  });
});

describe("settingsDialog update tab", () => {
  it("reports the update tab to the caller and shows a dot when an update exists", () => {
    const onUpdateSeen = vi.fn();
    const { container } = mount({ hasUpdate: true, onUpdateSeen });

    expect(navButton(container, "Check for updates").querySelector("span.bg-accent")).not.toBeNull();

    act(() => navButton(container, "Check for updates").click());
    expect(onUpdateSeen).toHaveBeenCalledTimes(1);
    expect(page(container)).toBe("update");
  });

  it("shows no dot without an update, and tolerates a missing onUpdateSeen", () => {
    const { container } = mount();
    expect(navButton(container, "Check for updates").querySelector("span.bg-accent")).toBeNull();

    // No onUpdateSeen supplied: navigating must not throw.
    act(() => navButton(container, "Check for updates").click());
    expect(page(container)).toBe("update");

    // Only the update tab notifies; another tab must not.
    const onUpdateSeen = vi.fn();
    const second = mount({ onUpdateSeen });
    act(() => navButton(second.container, "About").click());
    expect(onUpdateSeen).not.toHaveBeenCalled();
  });

  it("passes the cached update status down", () => {
    const cachedUpdateStatus: UpdateStatus = {
      currentVersion: "1.0.0",
      latestVersion: "1.1.0",
      hasUpdate: true,
      platformSupported: true,
      canInstallInApp: true,
      downloadUrl: "https://example.com/app.dmg",
    };
    const { container } = mount({ cachedUpdateStatus });
    act(() => navButton(container, "Check for updates").click());

    expect(container.querySelector("[data-page=update]")!.getAttribute("data-cached")).toBe("true");
  });

  it("marks the update nav item without a dot when the cached status says up to date", () => {
    const { container } = mount({ hasUpdate: false });

    const nav = navButton(container, "Check for updates");
    expect(nav.querySelector("span.bg-accent")).toBeNull();
    expect(nav.querySelector("svg")).not.toBeNull();
  });
});

describe("settingsDialog settings wiring", () => {
  it("maps every General toggle to its settings patch", async () => {
    const onChangeSettings = vi.fn().mockResolvedValue(undefined);
    const { container } = mount({ onChangeSettings });

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-approval-manual]")!.click();
      container.querySelector<HTMLButtonElement>("[data-toggle-title]")!.click();
      container.querySelector<HTMLButtonElement>("[data-toggle-bell]")!.click();
      container.querySelector<HTMLButtonElement>("[data-toggle-upgrade]")!.click();
      container.querySelector<HTMLButtonElement>("[data-toggle-recommend]")!.click();
    });

    expect(onChangeSettings.mock.calls.map(([patch]) => patch)).toEqual([
      { approvalTier: "manual" },
      { autoTitleFirstTurn: false },
      { bellOnComplete: false },
      { autoUpgradeSkills: true },
      { skillRecommend: false },
    ]);
  });

  it("forwards the stored settings into every page", () => {
    const { container } = mount();

    const general = container.querySelector("[data-page=general]")!;
    expect(general.getAttribute("data-approval")).toBe("manual");
    expect(general.getAttribute("data-auto-title")).toBe("false");
    expect(general.getAttribute("data-bell")).toBe("false");
    expect(general.getAttribute("data-auto-upgrade")).toBe("true");
    expect(general.getAttribute("data-skill-recommend")).toBe("false");

    act(() => navButton(container, "Phone Control").click());
    expect(container.querySelector("[data-page=remote]")!.getAttribute("data-auto-connect")).toBe("true");
  });

  it("maps the Remote toggle to autoConnectRemote", async () => {
    const onChangeSettings = vi.fn().mockResolvedValue(undefined);
    const { container } = mount({ onChangeSettings });

    act(() => navButton(container, "Phone Control").click());
    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-toggle-auto-connect]")!.click();
    });

    expect(onChangeSettings).toHaveBeenCalledExactlyOnceWith({ autoConnectRemote: true });
  });

  it("wires the account page to the shared account state and both refresh callbacks", () => {
    const onRefreshFutureAuth = vi.fn();
    const onRefreshFutureBalance = vi.fn();
    const { container } = mount({ futureBalance: 42.75, futureSessionStatus: "unavailable", onRefreshFutureAuth, onRefreshFutureBalance });

    act(() => navButton(container, "Account").click());
    const account = container.querySelector("[data-page=account]")!;
    expect(account.getAttribute("data-balance")).toBe("42.75");
    expect(account.getAttribute("data-balance-status")).toBe("available");
    expect(account.getAttribute("data-email")).toBe("user@example.com");
    expect(account.getAttribute("data-session")).toBe("unavailable");
    expect(account.getAttribute("data-community")).toBe("true");

    act(() => container.querySelector<HTMLButtonElement>("[data-refresh-auth]")!.click());
    act(() => container.querySelector<HTMLButtonElement>("[data-refresh-balance]")!.click());
    expect(onRefreshFutureAuth).toHaveBeenCalledTimes(1);
    expect(onRefreshFutureBalance).toHaveBeenCalledTimes(1);
  });

  it("maps the model visibility change onto hiddenModels", async () => {
    const onChangeSettings = vi.fn().mockResolvedValue(undefined);
    const { container } = mount({ onChangeSettings });

    act(() => navButton(container, "Models").click());
    expect(container.querySelector("[data-page=models]")!.getAttribute("data-hidden")).toBe("deepseek/deepseek-chat");
    expect(container.querySelector("[data-page=models]")!.getAttribute("data-options")).toBe("1");

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-hide-model]")!.click();
    });

    expect(onChangeSettings).toHaveBeenCalledExactlyOnceWith({ hiddenModels: ["future/gpt-5"] });
  });

  it("wires the providers page to the account gate and forwards its change signal", () => {
    const onProvidersChanged = vi.fn();
    const onRefreshFutureAuth = vi.fn();
    const { container } = mount({ onProvidersChanged, onRefreshFutureAuth, futureSessionStatus: "signed_out" });

    act(() => navButton(container, "Providers").click());
    const providers = container.querySelector("[data-page=providers]")!;
    expect(providers.getAttribute("data-session")).toBe("signed_out");
    expect(providers.getAttribute("data-community")).toBe("true");

    act(() => container.querySelector<HTMLButtonElement>("[data-providers-changed]")!.click());
    expect(onProvidersChanged).toHaveBeenCalledTimes(1);

    act(() => container.querySelector<HTMLButtonElement>("[data-refresh-providers-auth]")!.click());
    expect(onRefreshFutureAuth).toHaveBeenCalledTimes(1);
  });

  it("tolerates a missing onProvidersChanged", () => {
    const { container } = mount();

    act(() => navButton(container, "Providers").click());
    expect(() => act(() => container.querySelector<HTMLButtonElement>("[data-providers-changed]")!.click())).not.toThrow();
  });

  it("maps the community-edition switch onto communityEdition", async () => {
    const onChangeSettings = vi.fn().mockResolvedValue(undefined);
    const { container } = mount({ onChangeSettings });

    act(() => navButton(container, "Environment").click());
    expect(container.querySelector("[data-page=community-edition]")!.getAttribute("data-community")).toBe("true");

    await act(async () => {
      container.querySelector<HTMLButtonElement>("[data-community-switch]")!.click();
    });

    expect(onChangeSettings).toHaveBeenCalledExactlyOnceWith({ communityEdition: true });
  });
});
