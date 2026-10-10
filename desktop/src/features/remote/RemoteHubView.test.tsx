// @vitest-environment jsdom
import type { RemoteStatus } from "../remote/remoteClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RemoteHubView } from "./RemoteHubView";

/**
 * The Remote page's own contract: that it puts both directions in one place,
 * that it says in words which is which, and that each direction receives what it
 * needs.
 *
 * The two directions are stubbed because their internals are their own; what
 * belongs to *this* component is the page shell, the lead that distinguishes
 * local from remote, and the wiring.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const sections = vi.hoisted(() => ({
  host: null as unknown,
  client: null as unknown,
}));

vi.mock("../remote/RemoteView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteHostSection: (props: unknown) => {
      sections.host = props;
      return createElement("div", { "data-child": "host-section" });
    },
  };
});
vi.mock("../remote-peer/RemotePeersView", async () => {
  const { createElement } = await import("react");
  return {
    RemoteClientSection: (props: unknown) => {
      sections.client = props;
      return createElement("div", { "data-child": "client-section" });
    },
  };
});

const startWindowDrag = vi.fn();
vi.mock("../../lib/windowDrag", () => ({
  startWindowDrag: () => startWindowDrag(),
}));
vi.mock("../../lib/useIsFullscreen", () => ({ useIsFullscreen: () => false }));

function status(overrides: Partial<RemoteStatus> = {}): RemoteStatus {
  return {
    agentAvailable: true,
    desktopId: "desk_1",
    desktopPublicKey: "pk",
    natsUrl: "nats://example",
    pairId: "",
    pairingCode: null,
    pairingCodeExpiresAt: null,
    phase: "stopped",
    reason: null,
    recovery: null,
    warningCode: null,
    webLanUrl: null,
    webUrl: null,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function mount(overrides: Partial<Parameters<typeof RemoteHubView>[0]> = {}) {
  const props = {
    autoConnect: false,
    leftPanelExpanded: true,
    onRefreshRemote: vi.fn(async () => {}),
    onStartConversation: vi.fn(),
    onToggleAutoConnect: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    remoteStatus: status(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(RemoteHubView, props));
  });
  return props;
}

beforeEach(() => {
  sections.host = null;
  sections.client = null;
  startWindowDrag.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** The text of one labelled box, so an assertion cannot pass on the other's copy. */
function sectionText(testId: string): string {
  return container.querySelector(`[data-testid="${testId}"]`)?.textContent ?? "";
}

it("shows both directions on one page", async () => {
  await mount();

  expect(container.querySelector("[data-child='host-section']")).not.toBeNull();
  expect(container.querySelector("[data-child='client-section']")).not.toBeNull();
});

/**
 * The lead is the whole reason the two live together: the entries used to be two
 * rail labels a word apart, and nothing told the user which way each connected.
 */
it("says up front what is local and what is remote", async () => {
  await mount();

  const lead = container.querySelector("[data-testid='remote-hub-lead']")!.textContent ?? "";
  expect(lead).toContain("This computer's own conversations are in Chats");
  expect(lead).toContain("both directions");
});

it("names each direction, with its own explanation", async () => {
  await mount();

  const host = sectionText("remote-hub-host");
  expect(host).toContain("Let other devices connect here");
  // Each box carries the sentence that explains *that* direction, so a reader
  // who only scans headings still learns which side is which.
  expect(host).toContain("tools still run here");

  const client = sectionText("remote-hub-client");
  expect(client).toContain("Connect to another computer");
  expect(client).toContain("its tools run there");
});

/**
 * The headings, and only the headings.
 *
 * The explanatory sentences *should* mention phones — a phone is one of the
 * devices that can connect here, and leaving it out would be less clear, not
 * more neutral. What must not name a device is the label the user navigates by,
 * because either direction now works between two computers.
 */
it("names the directions by direction, not by the device on the other end", async () => {
  await mount();

  const headings = [...container.querySelectorAll("h2")].map(node => node.textContent ?? "");
  expect(headings).toEqual(["Let other devices connect here", "Connect to another computer"]);
  for (const heading of headings) {
    for (const device of ["Phone", "phone", "Mobile", "mobile"]) {
      expect(heading).not.toContain(device);
    }
  }
});

it("gives each direction what it needs, and nothing crossed over", async () => {
  const props = await mount({ autoConnect: true, remoteStatus: status({ pairId: "pair_1" }) });

  expect(sections.host).toMatchObject({
    autoConnect: true,
    remoteStatus: { pairId: "pair_1" },
  });
  expect(sections.client).toMatchObject({ onStartConversation: props.onStartConversation });
  // The host half has no machines to open conversations on, and the client half
  // has no pairing state: a prop on the wrong side would be invisible until it
  // misbehaved.
  expect(sections.host).not.toHaveProperty("onStartConversation");
  expect(sections.client).not.toHaveProperty("remoteStatus");
});

it("carries the page shell: the title and the window drag", async () => {
  await mount();

  expect(container.querySelector("header")!.textContent).toContain("Remote");

  await act(async () => {
    container.querySelector("header")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  });
  expect(startWindowDrag).toHaveBeenCalledTimes(1);
});
