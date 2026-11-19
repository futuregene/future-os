// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { ChoiceCardsProvider } from "../markdown/ChoiceCardsContext";
import { useRemoteChoiceCards } from "./useRemoteChoiceCards";

/**
 * Which switch a remote conversation's cards follow.
 *
 * It is the **host's**: the same value decides whether that machine's agent is
 * taught the fence at all, so a client that answered for itself would draw cards
 * the host never writes, or hide the ones it does — with the fence's own source
 * gone with them. Three answers have to be told apart:
 *
 * - `false` from the host → draw nothing;
 * - `true` → draw, whatever this machine says;
 * - **no answer at all** (a build older than the switch, which teaches the fence
 *   unconditionally) → draw, because "silent" is not "off";
 * - a refused read → this machine's own switch, because an unreachable host is
 *   not a host that said no.
 */

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));

/** The host's answer, by id; an `Error` is a host that refused to answer. */
const answers = new Map<string, unknown>();

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation(async (_name: string, args: { desktopId?: string; command?: { type?: string } }) => {
    if (args?.command?.type === "get_desktop_settings") {
      const answer = answers.get(args.desktopId ?? "");
      if (answer instanceof Error)
        throw answer;
      if (answer === undefined)
        throw new Error(`no settings for ${String(args.desktopId)}`);
      return answer;
    }
    return {};
  });
});

/** A reader, as the value a conversation would then draw with. */
async function resolve(desktopId: string, local: boolean): Promise<string> {
  function Probe() {
    return createElement("span", { "data-value": String(useRemoteChoiceCards(desktopId)) });
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(createElement(ChoiceCardsProvider, { value: local }, createElement(Probe)));
  });
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
  const value = container.querySelector("span")!.dataset.value;
  await act(async () => root.unmount());
  container.remove();
  return value ?? "missing";
}

it("follows the host's off even when this machine draws cards", async () => {
  answers.set("host_off", { choiceCards: false });
  expect(await resolve("host_off", true)).toBe("false");
});

it("follows the host's on even when this machine does not", async () => {
  answers.set("host_on", { choiceCards: true });
  expect(await resolve("host_on", false)).toBe("true");
});

it("reads a host that never answers the question as on", async () => {
  // A build older than the switch: it teaches the fence, so its replies do carry
  // cards and hiding them would delete a question it legitimately asks.
  answers.set("host_old", { skillRecommend: false });
  expect(await resolve("host_old", false)).toBe("true");
});

it("keeps this machine's own switch when the host cannot be asked", async () => {
  answers.set("host_unreachable", new Error("peer unreachable"));
  expect(await resolve("host_unreachable", false)).toBe("false");
  answers.set("host_unreachable_on", new Error("peer unreachable"));
  expect(await resolve("host_unreachable_on", true)).toBe("true");
});

it("has nothing of the host's to say before a conversation picks a machine", async () => {
  expect(await resolve("", false)).toBe("false");
  expect(await resolve("", true)).toBe("true");
});
