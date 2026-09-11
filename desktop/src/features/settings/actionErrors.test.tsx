// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AccountPage } from "./AccountPage";
import { CommunityEditionSection } from "./CommunityEditionSection";

vi.mock("../../integrations/agent/providers", () => ({
  listAgentProviders: async () => ({ builtin: [{ id: "future", hasApiKey: true }], custom: [] }),
  peekAgentProviders: () => ({ builtin: [{ id: "future", hasApiKey: true }], custom: [] }),
  getFutureEnvironment: async () => ({ platformUrl: "https://example.invalid" }),
  logoutFutureProvider: async () => { throw new Error("logout unavailable"); },
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("renders logout rejection without losing the confirmation controls", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<AccountPage balance={null} communityEdition={false} email={null} onRefreshBalance={() => {}} />));
    const click = async (label: string) => {
      const button = [...container.querySelectorAll("button")].find(item => item.textContent === label);
      expect(button, label).toBeTruthy();
      await act(async () => button!.click());
    };
    await click("Sign out");
    await click("Sign out");
    expect(container.querySelector("[role=alert]")?.textContent).toBe("logout unavailable");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});

it("reports a community mode save rejection and releases its busy state", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    const onChange = async () => {
      throw new Error("save unavailable");
    };
    await act(async () => root.render(<CommunityEditionSection communityEdition={false} onChangeCommunityEdition={onChange} />));
    act(() => container.querySelector<HTMLButtonElement>("[role=switch]")!.click());
    const button = [...container.querySelectorAll("button")].find(item => item.getAttribute("role") !== "switch")!;
    await act(async () => button.click());
    expect(container.querySelector("[role=alert]")?.textContent).toBe("save unavailable");
    expect(button.disabled).toBe(false);
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
