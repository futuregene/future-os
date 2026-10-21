// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteApprovalMenu } from "./RemoteApprovalMenu";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The approval mode the *host* will run this conversation under.
 *
 * Offered beside the box whose messages it governs, not in the settings page,
 * where it would read as a property of the machine rather than of the next thing
 * sent — which is where this app's own approval control is.
 */
const read = vi.fn<(desktopId: string) => Promise<{ approvalTier: string; sandboxAvailable: boolean }>>();
const set = vi.fn<(desktopId: string, tier: string) => Promise<string>>();
vi.mock("./remotePeerClient", () => ({
  getRemoteApprovalSettings: (...args: Parameters<typeof read>) => read(...args),
  setRemoteApprovalTier: (...args: Parameters<typeof set>) => set(...args),
}));

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
  desktopId: "desktop_a",
  error: null,
  features: ["auto_approval_v1"],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

beforeEach(() => {
  read.mockReset().mockResolvedValue({ approvalTier: "manual", sandboxAvailable: true });
  set.mockReset().mockImplementation(async (_desktopId, tier) => tier);
});

async function mount(props: { features?: string[] } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChanged = vi.fn();
  await act(async () => {
    root.render(
      <RemoteApprovalMenu
        desktopId="desktop_a"
        onChanged={onChanged}
        peer={{ ...peer, features: props.features ?? peer.features }}
      />,
    );
  });
  const trigger = () => container.querySelector<HTMLButtonElement>("button[title='Approval mode']")!;
  const item = (label: string) => [...container.querySelectorAll("button")].find(node => node.textContent?.includes(label))!;
  return {
    container,
    item,
    onChanged,
    trigger,
    open: async () => {
      await act(async () => trigger().click());
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("shows the mode the host reports, and settles on the one it accepts", async () => {
  const view = await mount();
  expect(read).toHaveBeenCalledWith("desktop_a");
  expect(view.trigger().textContent).toContain("Manual");

  // A host that answers `manual` to a request for `sandbox` is shown as `manual`:
  // that is the mode it is actually in.
  set.mockResolvedValue("manual");
  await view.open();
  await act(async () => view.item("Sandboxed")!.click());
  expect(set).toHaveBeenCalledWith("desktop_a", "sandbox");
  expect(view.trigger().textContent).toContain("Manual");
  expect(view.onChanged).toHaveBeenCalled();
  await view.unmount();
});

/** A refusal leaves the control showing the host's own mode. */
it("keeps the host's mode when the request fails", async () => {
  set.mockRejectedValue(new Error("refused"));
  const view = await mount();
  await view.open();
  await act(async () => view.item("Full access")!.click());
  expect(view.trigger().textContent).toContain("Manual");
  await view.unmount();
});

/** A host too old to report its settings has no control to offer. */
it("offers nothing when the host cannot report its settings", async () => {
  read.mockRejectedValue(new Error("unknown command"));
  const view = await mount();
  expect(view.container.querySelector("button[title]")).toBeNull();
  await view.unmount();
});

/**
 * Sandbox is refused by a host that says it has none, and automatic review by one
 * that does not declare it — the host's own answer about itself, not a guess.
 */
it("disables the modes the host cannot run", async () => {
  read.mockResolvedValue({ approvalTier: "manual", sandboxAvailable: false });
  const view = await mount({ features: [] });
  await view.open();

  const sandbox = view.item("Sandboxed");
  expect(sandbox.disabled).toBe(true);
  expect(view.item("Automatic review")!.disabled).toBe(true);
  // And the reason is given, so a disabled row is not a mystery.
  expect(view.container.textContent).toContain("Sandboxed (unavailable)");
  await view.unmount();
});

it("closes the menu on a second click of the trigger", async () => {
  const view = await mount();
  // The panel is this app's `SelectMenu`, which draws no `role` of its own, so
  // "open" is read from a row that only exists inside it.
  // A row carries its label *and* its description, so the row is found by the
  // label's presence rather than by the whole text.
  const open = () => [...view.container.querySelectorAll("button")].some(node => node.textContent?.includes("Full access"));
  await view.open();
  expect(open()).toBe(true);
  await act(async () => view.trigger().click());
  expect(open()).toBe(false);
  await view.unmount();
});

/** Dismissing the menu with Escape leaves the host's mode alone. */
it("closes on dismissal without asking the host for anything", async () => {
  const view = await mount();
  await view.open();
  expect(view.container.querySelectorAll("button").length).toBeGreaterThan(1);

  await act(async () => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
  expect(view.container.querySelectorAll("button")).toHaveLength(1);
  expect(set).not.toHaveBeenCalled();
  await view.unmount();
});

/**
 * A read that lands after the control is gone changes nothing.
 *
 * The panel is unmounted whenever the conversation closes, and a late answer must
 * not resurrect a control for a conversation the reader has left.
 */
it("ignores a settings read that settles after unmount", async () => {
  let resolve: (value: { approvalTier: string; sandboxAvailable: boolean }) => void = () => {};
  read.mockReturnValue(new Promise((done) => {
    resolve = done;
  }));
  const view = await mount();
  // Nothing to show until the host has answered.
  expect(view.container.querySelector("button[title='Approval mode']")).toBeNull();

  await view.unmount();
  await act(async () => resolve({ approvalTier: "off", sandboxAvailable: true }));
});
