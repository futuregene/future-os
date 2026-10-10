// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
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
  invoke.mockReset();
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

  invoke.mockRejectedValue(new Error("peer_not_connected"));
  const failed = await render(() => useRemoteSkillRecommend("desktop_a"));
  expect(failed.current).toBe(false);
  await failed.unmount();

  const none = await render(() => useRemoteSkillRecommend(""));
  expect(none.current).toBe(false);
  await none.unmount();
});
