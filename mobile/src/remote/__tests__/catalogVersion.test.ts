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

test("legacy Desktop retains arrival fencing, malformed versions never advance state", () => {
  const gate = new CatalogVersionGate();
  expect(gate.accept("sessions")).toBe(true);
  gate.authenticate("A");
  for (const revision of [NaN, Infinity, -1, 1.2, Number.MAX_SAFE_INTEGER + 1]) {
    expect(gate.accept("sessions", { epoch: "A", revision })).toBe(false);
  }
  expect(gate.accept("sessions", { epoch: "A", revision: 1 })).toBe(true);
});
