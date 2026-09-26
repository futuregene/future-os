// @vitest-environment jsdom
import type { FutureBalanceStatus, FutureSessionStatus } from "../../components/layout/hooks/useFutureAccount";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { AccountPage } from "./AccountPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  getFutureEnvironment: vi.fn(),
  logoutFutureProvider: vi.fn(),
  openExternalUrl: vi.fn(),
}));

vi.mock("../../integrations/agent/providers", () => ({
  getFutureEnvironment: mocks.getFutureEnvironment,
  logoutFutureProvider: mocks.logoutFutureProvider,
}));
vi.mock("../../integrations/storage/files", () => ({ openExternalUrl: mocks.openExternalUrl }));

const PRODUCTION = { environment: "production", platformUrl: "https://platform.example.com" };

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.getFutureEnvironment.mockReset();
  mocks.getFutureEnvironment.mockResolvedValue(PRODUCTION);
  mocks.logoutFutureProvider.mockReset();
  mocks.logoutFutureProvider.mockResolvedValue({ builtin: [], custom: [] });
  mocks.openExternalUrl.mockReset();
  mocks.openExternalUrl.mockResolvedValue(undefined);
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

interface PageOverrides {
  balance?: number | null;
  balanceStatus?: FutureBalanceStatus;
  communityEdition?: boolean;
  email?: string | null;
  onRefreshAuth?: () => void;
  onRefreshBalance?: () => void;
  sessionStatus?: FutureSessionStatus;
}

async function renderPage(overrides: PageOverrides = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(
      <AccountPage
        balance={overrides.balance ?? null}
        balanceStatus={overrides.balanceStatus ?? "available"}
        communityEdition={overrides.communityEdition ?? false}
        email={overrides.email === undefined ? "user@example.com" : overrides.email}
        sessionStatus={overrides.sessionStatus ?? "authenticated"}
        onRefreshAuth={overrides.onRefreshAuth ?? (() => {})}
        onRefreshBalance={overrides.onRefreshBalance ?? (() => {})}
      />,
    );
  });
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement | undefined;
}

function alertText(container: HTMLElement) {
  return container.querySelector("[role=alert]")?.textContent ?? null;
}

/** The description sub-text of the first settings row (the account row). */
function accountDescription(container: HTMLElement) {
  const row = container.querySelector("[class*='py-3.5']")!;
  return row.querySelector("[class*='mt-0.5']")?.textContent ?? null;
}

describe("accountPage session labels", () => {
  it.each([
    ["checking", null, "Checking sign-in…"],
    ["checking", "user@example.com", "user@example.com · Checking sign-in…"],
    ["invalid", "user@example.com", "Sign-in expired"],
    ["signed_out", "user@example.com", "Not signed in"],
    ["unavailable", null, "Account temporarily unavailable"],
    ["unavailable", "user@example.com", "user@example.com · Account temporarily unavailable"],
    ["authenticated", "user@example.com", "user@example.com"],
    // No email yet: the row renders no description at all rather than flashing a
    // generic "Signed in" label (the row collapses to its title).
    ["authenticated", null, null],
  ] as const)("describes a %s session with email %j", async (sessionStatus, email, expected) => {
    const container = await renderPage({ sessionStatus, email });

    expect(accountDescription(container)).toBe(expected);
  });

  it("offers no account action while the session is still being checked", async () => {
    const container = await renderPage({ sessionStatus: "checking" });

    expect(container.querySelectorAll("button")).toHaveLength(0);
  });

  it("asks for a fresh sign-in on an expired session", async () => {
    const events: unknown[] = [];
    const off = onFutureEvent("show-onboarding", () => events.push("onboarding"));
    try {
      const container = await renderPage({ sessionStatus: "invalid" });

      expect(button(container, "Sign in again")).toBeTruthy();
      act(() => button(container, "Sign in again")!.click());
      expect(events).toEqual(["onboarding"]);
    }
    finally {
      off();
    }
  });

  it("offers a plain sign-in when signed out", async () => {
    const container = await renderPage({ sessionStatus: "signed_out" });

    expect(button(container, "Sign in")).toBeTruthy();
    expect(button(container, "Sign out")).toBeUndefined();
  });
});

describe("accountPage authenticated actions", () => {
  it("opens the platform account page in the system browser", async () => {
    const container = await renderPage();

    await act(async () => {
      button(container, "View account info")!.click();
    });

    expect(mocks.openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://platform.example.com/platform/");
  });

  it("opens the recharge page from the balance row", async () => {
    const container = await renderPage({ balance: 42.75 });

    expect(container.textContent).toContain("42 credits");

    await act(async () => {
      button(container, "Recharge")!.click();
    });

    expect(mocks.openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://platform.example.com/platform/#recharge");
  });

  it("signs out through the confirmation row", async () => {
    const container = await renderPage();

    act(() => button(container, "Sign out")!.click());
    expect(container.textContent).toContain("Sign out?");

    act(() => button(container, "Cancel")!.click());
    expect(mocks.logoutFutureProvider).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Sign out?");

    act(() => button(container, "Sign out")!.click());
    await act(async () => {
      button(container, "Sign out")!.click();
    });

    expect(mocks.logoutFutureProvider).toHaveBeenCalledTimes(1);
    expect(container.textContent).not.toContain("Sign out?");
    expect(alertText(container)).toBeNull();
  });

  it("reports a failed sign-out and keeps the confirmation row", async () => {
    mocks.logoutFutureProvider.mockRejectedValue(new Error("platform unreachable"));
    const container = await renderPage();

    act(() => button(container, "Sign out")!.click());
    await act(async () => {
      button(container, "Sign out")!.click();
    });

    expect(alertText(container)).toBe("platform unreachable");
    expect(container.textContent).toContain("Sign out?");
  });

  it("stringifies a non-Error sign-out failure", async () => {
    mocks.logoutFutureProvider.mockRejectedValue("ipc gone");
    const container = await renderPage();

    act(() => button(container, "Sign out")!.click());
    await act(async () => {
      button(container, "Sign out")!.click();
    });

    expect(alertText(container)).toBe("ipc gone");
  });
});

describe("accountPage unavailable session", () => {
  it("cancels the confirmation in the unavailable branch", async () => {
    const container = await renderPage({ sessionStatus: "unavailable" });

    act(() => button(container, "Sign out")!.click());
    expect(container.textContent).toContain("Sign out?");

    act(() => button(container, "Cancel")!.click());

    expect(container.textContent).not.toContain("Sign out?");
    expect(button(container, "Retry")).toBeTruthy();
    expect(mocks.logoutFutureProvider).not.toHaveBeenCalled();
  });

  it("offers a retry and a confirmed sign-out", async () => {
    const onRefreshAuth = vi.fn();
    const container = await renderPage({ sessionStatus: "unavailable", onRefreshAuth });

    expect(button(container, "View account info")).toBeUndefined();
    act(() => button(container, "Retry")!.click());
    expect(onRefreshAuth).toHaveBeenCalledTimes(1);

    act(() => button(container, "Sign out")!.click());
    expect(container.textContent).toContain("Sign out?");
    await act(async () => {
      button(container, "Sign out")!.click();
    });
    expect(mocks.logoutFutureProvider).toHaveBeenCalledTimes(1);
  });
});

describe("accountPage balance states", () => {
  it.each([
    ["loading", null, "Refreshing…"],
    ["unavailable", null, "Balance temporarily unavailable"],
    ["available", null, "—"],
    ["available", 0, "0 credits"],
    ["available", 7.9, "7 credits"],
    ["idle", 3, "3 credits"],
  ] as const)("shows %s / %j as %j", async (balanceStatus, balance, expected) => {
    const container = await renderPage({ balanceStatus, balance });

    expect(container.textContent).toContain(expected);
  });

  it("retries the balance instead of recharging while it is unavailable", async () => {
    const onRefreshBalance = vi.fn();
    const container = await renderPage({ balanceStatus: "unavailable", onRefreshBalance });

    await act(async () => {
      button(container, "Retry")!.click();
    });

    expect(onRefreshBalance).toHaveBeenCalled();
    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("disables the recharge button until the platform URL is known", async () => {
    let resolveEnv!: (value: unknown) => void;
    mocks.getFutureEnvironment.mockReturnValue(new Promise((resolve) => {
      resolveEnv = resolve;
    }));
    const container = await renderPage({ balance: 5 });

    expect(button(container, "Recharge")!.disabled).toBe(true);
    expect(button(container, "View account info")!.disabled).toBe(true);

    await act(async () => {
      resolveEnv(PRODUCTION);
    });
    expect(button(container, "Recharge")!.disabled).toBe(false);
  });

  it("does nothing when the environment lookup failed (no platform URL)", async () => {
    mocks.getFutureEnvironment.mockRejectedValue(new Error("agent offline"));
    const container = await renderPage({ balance: 5 });

    // The failed lookup is silent on this page; the balance is still shown and
    // both platform-dependent actions stay disabled.
    expect(container.textContent).toContain("5 credits");
    expect(button(container, "Recharge")!.disabled).toBe(true);
    expect(button(container, "View account info")!.disabled).toBe(true);

    // Fault injection: drop React's disabled gate so the handlers themselves run
    // and must refuse to open a URL built from an undefined platform root.
    const recharge = button(container, "Recharge")!;
    const viewInfo = button(container, "View account info")!;
    recharge.removeAttribute("disabled");
    viewInfo.removeAttribute("disabled");
    await act(async () => {
      recharge.click();
      viewInfo.click();
    });

    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("keeps retry enabled when the platform URL is unknown but the balance is unavailable", async () => {
    mocks.getFutureEnvironment.mockRejectedValue(new Error("agent offline"));
    const onRefreshBalance = vi.fn();
    const container = await renderPage({ balanceStatus: "unavailable", onRefreshBalance });

    expect(button(container, "Retry")!.disabled).toBe(false);
    await act(async () => {
      button(container, "Retry")!.click();
    });
    expect(onRefreshBalance).toHaveBeenCalled();
  });
});

describe("accountPage refresh and community edition", () => {
  it("treats an empty platform URL as unknown for the account link", async () => {
    // The account row is gated on `environment.data` being present, so a
    // resolved-but-empty platform root reaches the handler's own guard instead
    // of being pre-empted by `disabled`.
    mocks.getFutureEnvironment.mockResolvedValue({ environment: "custom", platformUrl: "" });
    const container = await renderPage({ balance: 5 });

    expect(button(container, "View account info")!.disabled).toBe(false);
    await act(async () => {
      button(container, "View account info")!.click();
    });

    expect(mocks.openExternalUrl).not.toHaveBeenCalled();
  });

  it("refreshes the balance once for an authenticated session", async () => {
    const onRefreshBalance = vi.fn();
    await renderPage({ onRefreshBalance });

    expect(onRefreshBalance).toHaveBeenCalledTimes(1);
  });

  it("also refreshes for an unavailable session (the cached balance may be stale)", async () => {
    const onRefreshBalance = vi.fn();
    await renderPage({ sessionStatus: "unavailable", onRefreshBalance });

    expect(onRefreshBalance).toHaveBeenCalledTimes(1);
  });

  it("does not refresh while signed out", async () => {
    const onRefreshBalance = vi.fn();
    await renderPage({ sessionStatus: "signed_out", onRefreshBalance });

    expect(onRefreshBalance).not.toHaveBeenCalled();
  });

  it("hides the balance and skips the refresh in the community edition", async () => {
    const onRefreshBalance = vi.fn();
    const container = await renderPage({ communityEdition: true, balance: 100, onRefreshBalance });

    expect(container.textContent).not.toContain("Balance");
    expect(button(container, "Recharge")).toBeUndefined();
    expect(onRefreshBalance).not.toHaveBeenCalled();
    // The account actions themselves are still offered.
    expect(button(container, "View account info")).toBeTruthy();
  });
});
