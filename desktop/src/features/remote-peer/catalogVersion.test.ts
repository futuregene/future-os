import { beforeEach, describe, expect, it } from "vitest";
import { CatalogVersionGate } from "./catalogVersion";

describe("catalog version gate", () => {
  let gate: CatalogVersionGate;

  beforeEach(() => {
    gate = new CatalogVersionGate();
  });

  it("rejects old revision", () => {
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 2 });
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 1 })).toBe(false);
    expect(gate.revision("sessions")).toBe(2);
  });

  it("rejects same revision without allowEqual", () => {
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 2 });
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 2 })).toBe(false);
  });

  it("accepts same revision with allowEqual", () => {
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 2 });
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 2 }, true)).toBe(true);
  });

  it("accepts newer revision", () => {
    gate.authenticate("epoch_1");
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 1 })).toBe(true);
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 5 })).toBe(true);
    expect(gate.revision("sessions")).toBe(5);
  });

  it("rejects different epoch", () => {
    gate.authenticate("epoch_1");
    expect(gate.accept("sessions", { epoch: "epoch_2", revision: 1 })).toBe(false);
  });

  it("keeps the applied revisions when authenticate is called again with the same epoch", () => {
    // Presence repeats its epoch every tick (3 s). Re-authenticating on each
    // tick reset the ledger, so the revision comparison that follows always saw
    // the -1 floor and every heartbeat healed with a full catalogue read. The
    // phone twin short-circuits the same way.
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 5 });
    gate.authenticate("epoch_1");
    expect(gate.revision("sessions")).toBe(5);
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 5 })).toBe(false);
  });

  it("resets the applied revisions when the epoch changes", () => {
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 5 });
    // A re-pair/reconnect mints a new epoch; its revisions start over, so the
    // old ones must not shadow them.
    gate.authenticate("epoch_2");
    expect(gate.revision("sessions")).toBe(-1);
    expect(gate.accept("sessions", { epoch: "epoch_2", revision: 1 })).toBe(true);
  });

  it("tracks revisions per domain independently", () => {
    gate.authenticate("epoch_1");
    gate.accept("sessions", { epoch: "epoch_1", revision: 3 });
    gate.accept("workspaces", { epoch: "epoch_1", revision: 1 });
    expect(gate.revision("sessions")).toBe(3);
    expect(gate.revision("workspaces")).toBe(1);
  });

  it("is not authenticated before authenticate is called", () => {
    expect(gate.authenticated).toBe(false);
  });

  it("is authenticated after authenticate is called", () => {
    gate.authenticate("epoch_1");
    expect(gate.authenticated).toBe(true);
  });

  it("accepts undefined version when unauthenticated (old host)", () => {
    expect(gate.accept("sessions", undefined)).toBe(true);
  });

  it("rejects defined version when unauthenticated (epoch missing)", () => {
    expect(gate.accept("sessions", { epoch: "epoch_1", revision: 1 })).toBe(false);
  });

  it("rejects undefined version once authenticated", () => {
    gate.authenticate("epoch_1");
    expect(gate.accept("sessions", undefined)).toBe(false);
  });
});
