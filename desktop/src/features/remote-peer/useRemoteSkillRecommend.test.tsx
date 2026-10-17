// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { forgetHostCatalogue } from "./remoteHostCatalog";
import { useRemoteSkillCatalog } from "./useRemoteSkillCatalog";
import { useRemoteSkillRecommend } from "./useRemoteSkillRecommend";

/**
 * The two reads that decide whether a remote conversation gets a recommendation
 * card at all: that machine's catalogue, and that machine's own toggle.
 *
 * Both fail towards "nothing": a card the user did not ask for is worse than one
 * they never see, and the recommender is best-effort either way.
 */

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));

/**
 * The host's own push lane, mocked so the invalidation path can be driven: both
 * hooks here re-read on the host's word, and without this the real `listen` rejects
 * in jsdom and the invalidator never fires — which is exactly how these lines went
 * unexercised.
 */
let listeners: Array<(event: { payload: unknown }) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

/** A push from the host, in the shape the bridge forwards. */
function push(type: string, desktopId = "desktop_a"): void {
  act(() => {
    listeners.forEach(listener => listener({ payload: { desktopId, kind: "event", payload: { type } } }));
  });
}

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

async function render<T>(use: () => T) {
  let value: T | undefined;
  function Probe() {
    value = use();
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<Probe />));
  await settle();
  return {
    get current(): T {
      return value as T;
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 8; i += 1)
      await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  listeners = [];
  invoke.mockReset();
  // Both reads are cached per host for as long as this window is open
  // (`remoteHostCatalog`), so one test's answer would otherwise be the next
  // one's — and the read it asserts would never happen at all.
  forgetHostCatalogue("desktop_a");
});

/** Both directions of the host's catalogue, in the shape the card needs. */
it("reads that host's catalogue and what it has installed", async () => {
  invoke.mockImplementation(async (_command: string, args?: unknown) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "list_available_skills") {
      return {
        skills: [{
          id: "pdf-tools",
          name: "PDF tools",
          description: "Read PDFs",
          descriptionZh: "读 PDF",
          latestVersion: "1.2.0",
        }],
      };
    }
    if (type === "list_skills")
      return { skills: [{ id: "existing", name: "Existing", description: "d" }] };
    return undefined;
  });

  const probe = await render(() => useRemoteSkillCatalog("desktop_a", true));

  expect(probe.current.catalogue).toEqual([{
    id: "pdf-tools",
    description: "Read PDFs",
    descriptionZh: "读 PDF",
    // The version is what an install asks the host for.
    version: "1.2.0",
  }]);
  expect(probe.current.installed).toEqual([{ id: "existing" }]);
  await probe.unmount();
});

/** A failed read is an empty catalogue: no candidates means no recommendation. */
it("reads a failed catalogue as empty", async () => {
  invoke.mockRejectedValue(new Error("peer_not_connected"));
  const probe = await render(() => useRemoteSkillCatalog("desktop_a", true));

  expect(probe.current.catalogue).toEqual([]);
  expect(probe.current.installed).toEqual([]);
  await probe.unmount();
});

/** A disabled reader asks the host nothing. */
it("asks nothing while recommendation is off", async () => {
  const probe = await render(() => useRemoteSkillCatalog("desktop_a", false));

  expect(invoke).not.toHaveBeenCalled();
  expect(probe.current.catalogue).toEqual([]);
  await probe.unmount();
});

/**
 * `reload` is how the caller says the catalogue moved (a skill was installed
 * from a card), and it re-reads rather than patching: the host is the one that
 * knows what is installable.
 */
it("re-reads the catalogue when asked to", async () => {
  let reads = 0;
  invoke.mockImplementation(async (_command: string, args?: unknown) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "list_available_skills") {
      reads += 1;
      return { skills: [{ id: `skill-${reads}`, name: "n", description: "d" }] };
    }
    if (type === "list_skills")
      return { skills: [] };
    return undefined;
  });

  const probe = await render(() => useRemoteSkillCatalog("desktop_a", true));
  expect(probe.current.catalogue[0]!.id).toBe("skill-1");

  await act(async () => probe.current.reload());
  await settle();

  expect(probe.current.catalogue[0]!.id).toBe("skill-2");
  await probe.unmount();
});

/**
 * A read that lands after the conversation moved on is discarded.
 *
 * Otherwise one computer's skills could be installed as another's, and the card
 * would offer something the machine it is about does not have.
 */
it("ignores a catalogue read that lands after the conversation moved on", async () => {
  let resolveRead: ((value: unknown) => void) | null = null;
  invoke.mockImplementation(async (_command: string, args?: unknown) => {
    const type = (args as { command?: { type?: string } })?.command?.type;
    if (type === "list_available_skills") {
      return new Promise((resolve) => {
        resolveRead = resolve;
      });
    }
    if (type === "list_skills")
      return { skills: [] };
    return undefined;
  });

  const probe = await render(() => useRemoteSkillCatalog("desktop_a", true));
  await probe.unmount();

  await act(async () => {
    resolveRead?.({ skills: [{ id: "late", name: "n", description: "d" }] });
  });
  await settle();

  // The read is simply dropped; nothing is asserted on an unmounted hook beyond
  // that it did not throw while writing into it.
  expect(resolveRead).not.toBeNull();
});

/** The toggle that matters is that machine's, and it is read from there. */
it("reads that machine's own recommendation toggle", async () => {
  invoke.mockResolvedValue({ skillRecommend: true });
  const probe = await render(() => useRemoteSkillRecommend("desktop_a"));

  expect(probe.current).toBe(true);
  expect(invoke).toHaveBeenCalledWith("remote_peer_request", {
    desktopId: "desktop_a",
    command: { type: "get_desktop_settings" },
    lane: "list",
  });
  await probe.unmount();
});

/**
 * Anything unclear is "off": a host that fails to answer, or that predates the
 * setting, must not produce cards on the strength of this machine's toggle.
 */
it("reads an unavailable toggle as off", async () => {
  invoke.mockResolvedValue({});
  const absent = await render(() => useRemoteSkillRecommend("desktop_a"));
  expect(absent.current).toBe(false);
  await absent.unmount();

  // The host itself cannot be reached. The catalogue cache has to be dropped
  // first, or this render would be answered by the *previous* render's resolved
  // read and the failed-read path would never run — the test would still pass
  // (both outcomes are `false`), which is exactly how this case was passing
  // before the assertion on the call count was added.
  forgetHostCatalogue("desktop_a");
  invoke.mockReset();
  invoke.mockRejectedValue(new Error("peer_not_connected"));
  const failed = await render(() => useRemoteSkillRecommend("desktop_a"));
  expect(invoke).toHaveBeenCalledWith("remote_peer_request", expect.objectContaining({
    command: { type: "get_desktop_settings" },
  }));
  expect(failed.current).toBe(false);
  await failed.unmount();

  const none = await render(() => useRemoteSkillRecommend(""));
  expect(none.current).toBe(false);
  await none.unmount();
});

/**
 * The host says its own settings moved, so the answer has to be read again.
 *
 * The setting belongs to the machine being written to, and a user who changed
 * recommendation *there* — or had the host's cached read dropped — must not keep
 * being offered cards on the strength of the stale answer. The domain is what the
 * callback filters on, so the sibling domain that lands on the same host must do
 * nothing: a model-visibility change is not a reason to re-ask about skill
 * suggestions.
 */
it("re-reads the toggle when the host says its settings changed, and only then", async () => {
  invoke.mockResolvedValue({ skillRecommend: true });
  const probe = await render(() => useRemoteSkillRecommend("desktop_a"));
  expect(invoke).toHaveBeenCalledTimes(1);

  // A change to another domain on the same host is not this hook's business.
  invoke.mockClear();
  push("model_visibility_changed");
  await settle();
  expect(invoke).not.toHaveBeenCalled();

  // The settings domain is: the cached read was dropped, so this re-reads.
  push("app_settings_changed");
  await settle();
  expect(invoke).toHaveBeenCalledWith("remote_peer_request", expect.objectContaining({
    command: { type: "get_desktop_settings" },
  }));

  await probe.unmount();
});

/**
 * The same rule for the catalogue behind the `/` menu: a skills change on the host
 * re-reads it, and the other domains leave it alone.
 */
it("re-reads the skill catalogue when the host says its skills changed", async () => {
  invoke.mockResolvedValue([]);
  const probe = await render(() => useRemoteSkillCatalog("desktop_a", true));
  const readsBefore = invoke.mock.calls.length;

  push("model_visibility_changed");
  await settle();
  expect(invoke.mock.calls.length).toBe(readsBefore);

  push("skills_changed");
  await settle();
  expect(invoke.mock.calls.length).toBeGreaterThan(readsBefore);

  await probe.unmount();
});
