import type { CommandId } from "../keyboard/catalog";
import { describe, expect, it } from "vitest";
import { activeHints } from "./hints";

function registered(...ids: CommandId[]) {
  return {
    hasHandler: (id: CommandId) => ids.includes(id),
    keysOf: (id: CommandId) => (id === "vibe.home" ? "↩" : ""),
  };
}

describe("activeHints", () => {
  it("lists a command that has both a handler and a binding", () => {
    expect(activeHints(["vibe.home"], registered("vibe.home")))
      .toEqual([{ id: "vibe.home", keys: "↩" }]);
  });

  it("leaves out a command nobody registered", () => {
    // The layer is not open, so nothing holds the send key: promising it would
    // teach the reader a key that does nothing.
    expect(activeHints(["vibe.home"], registered())).toEqual([]);
  });

  it("leaves out a command the reader unbound", () => {
    const unbound = {
      hasHandler: () => true,
      keysOf: () => "",
    };
    expect(activeHints(["vibe.home"], unbound)).toEqual([]);
  });

  it("keeps the order it was asked for", () => {
    const deps = {
      hasHandler: () => true,
      keysOf: (id: CommandId) => (id === "vibe.input.close" ? "Esc" : "↩"),
    };
    expect(activeHints(["vibe.input.close", "vibe.home"], deps).map(item => item.keys))
      .toEqual(["Esc", "↩"]);
  });

  it("is empty for an empty list", () => {
    expect(activeHints([], registered("vibe.home"))).toEqual([]);
  });
});
