import { beforeEach, expect, it, vi } from "vitest";
import { remoteSkillRecoSource } from "./remoteSkillRecoSource";

/**
 * The recommendation data of a paired computer.
 *
 * Everything here is about *whose* answer is used: a card that installs a skill
 * on the wrong machine, or a budget counted against this one, would look
 * identical on screen.
 */

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: invoke }));

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
});

/** The host's own day state, and the command it comes from. */
it("reads that host's day state", async () => {
  invoke.mockResolvedValue({
    today: { count: 2, skillIds: ["a"], messageHashes: ["h1"] },
  });
  await expect(remoteSkillRecoSource("desktop_a").today()).resolves.toEqual({
    count: 2,
    skillIds: ["a"],
    messageHashes: ["h1"],
  });

  expect(invoke.mock.calls[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "skill_reco_today" },
    lane: "list",
  });
});

/**
 * A day state this machine cannot read is an *empty* day, not an error.
 *
 * The budget can then only be under-counted, which fails towards recommending
 * rather than towards silently disabling the feature.
 */
it("reads an unusable day state as an empty day", async () => {
  const source = remoteSkillRecoSource("desktop_a");

  invoke.mockRejectedValue(new Error("peer_not_connected"));
  await expect(source.today()).resolves.toEqual({ count: 0, skillIds: [], messageHashes: [] });

  invoke.mockResolvedValue({ today: "nonsense" });
  await expect(source.today()).resolves.toEqual({ count: 0, skillIds: [], messageHashes: [] });

  invoke.mockResolvedValue({ today: { count: 1, skillIds: [null, "a"], messageHashes: [7] } });
  await expect(source.today()).resolves.toEqual({ count: 1, skillIds: ["a"], messageHashes: [] });
});

/** The candidate set goes with the request: the host recommends from *its* skills. */
it("sends the candidates to that host", async () => {
  invoke.mockResolvedValue({ skill: { name: "pdf-tools", description: "Read PDFs" } });
  const candidates = [{ name: "pdf-tools", description: "Read PDFs" }];

  await expect(remoteSkillRecoSource("desktop_a").suggest("summarise a pdf", candidates))
    .resolves
    .toEqual({ name: "pdf-tools", description: "Read PDFs" });

  expect(invoke.mock.calls[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "suggest_skill", query: "summarise a pdf", candidates },
    lane: "list",
  });
});

/**
 * Every way the host can decline is the same answer.
 *
 * `skill: null` is what it sends for "not signed in there", "my recommender
 * timed out" and "no match" alike — a best-effort recommendation has no use for
 * telling them apart, and treating one as an error would surface a failure the
 * user cannot act on.
 */
it("reads every refusal as no recommendation", async () => {
  const source = remoteSkillRecoSource("desktop_a");

  invoke.mockResolvedValue({ skill: null });
  await expect(source.suggest("q", [])).resolves.toBeNull();

  invoke.mockResolvedValue({});
  await expect(source.suggest("q", [])).resolves.toBeNull();

  // A malformed entry is no recommendation either, rather than a card with no
  // skill behind its install button.
  invoke.mockResolvedValue({ skill: { description: "no name" } });
  await expect(source.suggest("q", [])).resolves.toBeNull();

  invoke.mockResolvedValue({ skill: "nonsense" });
  await expect(source.suggest("q", [])).resolves.toBeNull();
});

/** A skill with no description still recommends; the name is what installs it. */
it("keeps a recommendation whose description is missing", async () => {
  invoke.mockResolvedValue({ skill: { name: "pdf-tools" } });
  await expect(remoteSkillRecoSource("desktop_a").suggest("q", [])).resolves.toEqual({
    name: "pdf-tools",
    description: "",
  });
});

/** Recording a shown card is a write on that host's budget, not this one's. */
it("records a shown recommendation on that host", async () => {
  await remoteSkillRecoSource("desktop_a").record("pdf-tools", "hash_1");

  expect(invoke.mock.calls[0]![1]).toEqual({
    desktopId: "desktop_a",
    command: { type: "record_skill_reco", skillId: "pdf-tools", messageHash: "hash_1" },
    lane: "list",
  });
});

/**
 * This machine cannot see that machine's account, so it does not pretend to.
 *
 * The host answers with no recommendation when its own account cannot produce
 * one, which is the same outcome without a guess.
 */
it("does not gate on an account it cannot read", () => {
  expect(remoteSkillRecoSource("desktop_a").accountGated).toBe(false);
});
