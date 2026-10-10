import { CatalogVersionGate } from "../catalogVersion";

test("only an authenticated source can advance a catalog and old or duplicate snapshots are ignored", () => {
  const gate = new CatalogVersionGate();
  expect(gate.accept("sessions", { epoch: "A", revision: 1 })).toBe(false);
  gate.authenticate("A");
  expect(gate.accept("sessions", { epoch: "A", revision: 3 })).toBe(true);
  expect(gate.accept("sessions", { epoch: "A", revision: 2 })).toBe(false);
  expect(gate.accept("sessions", { epoch: "A", revision: 3 })).toBe(false);
  expect(gate.accept("sessions", { epoch: "B", revision: 100 })).toBe(false);
  expect(gate.accept("sessions")).toBe(false);
  expect(gate.accept("workspaces", { epoch: "A", revision: 1 })).toBe(true);
  gate.authenticate("B");
  expect(gate.accept("sessions", { epoch: "B", revision: 1 })).toBe(true);
  expect(gate.accept("sessions", { epoch: "A", revision: 999 })).toBe(false);
});

test("an explicit freshness read can repeat a revision but cannot regress or change epoch", () => {
  const gate = new CatalogVersionGate();
  gate.authenticate("A");
  expect(gate.accept("sessions", { epoch: "A", revision: 3 })).toBe(true);
  expect(gate.accept("sessions", { epoch: "A", revision: 3 }, true)).toBe(true);
  expect(gate.accept("sessions", { epoch: "A", revision: 3 })).toBe(false);
  expect(gate.accept("sessions", { epoch: "A", revision: 2 }, true)).toBe(false);
  expect(gate.accept("sessions", { epoch: "B", revision: 4 }, true)).toBe(false);
});

/**
 * `revision()` is what the presence heartbeat is compared against. If it
 * reported anything other than "the revision actually applied" — a rejected
 * snapshot counting, or a stale compare-and-set — the client would either
 * stop asking for a snapshot it never got or ask forever.
 */
test("revision() reports what was applied so a presence revision can be compared", () => {
  const gate = new CatalogVersionGate();
  expect(gate.revision("sessions")).toBe(-1);
  expect(gate.revision("workspaces")).toBe(-1);
  gate.authenticate("A");
  expect(gate.accept("sessions", { epoch: "A", revision: 7 })).toBe(true);
  expect(gate.revision("sessions")).toBe(7);
  expect(gate.accept("sessions", { epoch: "A", revision: 6 })).toBe(false);
  expect(gate.revision("sessions")).toBe(7);
  // Domains are independent.
  expect(gate.revision("workspaces")).toBe(-1);
  // Re-authenticating a new epoch clears the baseline.
  gate.authenticate("B");
  expect(gate.revision("sessions")).toBe(-1);
});

test("legacy Desktop retains arrival fencing, malformed versions never advance state", () => {
  const gate = new CatalogVersionGate();
  expect(gate.accept("sessions")).toBe(true);
  gate.authenticate("A");
  for (const revision of [NaN, Infinity, -1, 1.2, Number.MAX_SAFE_INTEGER + 1]) {
    expect(gate.accept("sessions", { epoch: "A", revision })).toBe(false);
  }
  expect(gate.accept("sessions", { epoch: "A", revision: 1 })).toBe(true);
});

test("re-authenticating within the same epoch keeps the revision fence", () => {
  const gate = new CatalogVersionGate();
  gate.authenticate("A");
  expect(gate.accept("sessions", { epoch: "A", revision: 5 })).toBe(true);
  // A reconnect that reports the same epoch must not let a replay of the same
  // snapshot through as if it were newer.
  gate.authenticate("A");
  expect(gate.accept("sessions", { epoch: "A", revision: 5 })).toBe(false);
  expect(gate.revision("sessions")).toBe(5);
  // A genuinely new epoch does start the counter over.
  gate.authenticate("B");
  expect(gate.revision("sessions")).toBe(-1);
  expect(gate.accept("sessions", { epoch: "B", revision: 5 })).toBe(true);
});
