import { afterEach, describe, expect, it, vi } from "vitest";
import { parseBinding } from "../../lib/keybinding";
import { clearCommandHandlers, hasCommandHandler, registerCommand, runCommand } from "./commands";

/**
 * The command registry's own contract: a keystroke is one lookup, and a command
 * nobody registered answers "not run" instead of throwing. Every caller that
 * dispatches a key depends on that second half — the dispatcher asks the registry
 * rather than a list of mounted components, so an unmounted command has to be a
 * quiet no-op.
 */
afterEach(() => {
  clearCommandHandlers();
});

describe("runCommand", () => {
  it("reports a command nobody registered rather than throwing", () => {
    expect(hasCommandHandler("chat.new")).toBe(false);
    expect(runCommand("chat.new")).toBe(false);
  });

  it("runs the registered handler and reports that it ran", () => {
    const run = vi.fn();
    registerCommand("chat.new", run);
    expect(hasCommandHandler("chat.new")).toBe(true);
    expect(runCommand("chat.new")).toBe(true);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("hands the pressed key through to the handler", () => {
    // The commands that answer to more than one key read which one was pressed
    // (`vibe.cell` picks the cell from the digit).
    const run = vi.fn();
    registerCommand("vibe.cell", run);
    const binding = parseBinding("1")!;
    runCommand("vibe.cell", binding);
    expect(run).toHaveBeenCalledWith(binding);
  });

  it("stops running a command as soon as its owner unregisters", () => {
    const run = vi.fn();
    const unregister = registerCommand("chat.new", run);
    unregister();
    expect(hasCommandHandler("chat.new")).toBe(false);
    expect(runCommand("chat.new")).toBe(false);
    // The unsubscriber is idempotent and never removes someone else's registration.
    unregister();
    expect(runCommand("chat.new")).toBe(false);
  });

  it("leaves a newer registration in place when the older owner unregisters", () => {
    const older = vi.fn();
    const newer = vi.fn();
    const unregisterOlder = registerCommand("chat.new", older);
    registerCommand("chat.new", newer);
    unregisterOlder();
    expect(runCommand("chat.new")).toBe(true);
    expect(older).not.toHaveBeenCalled();
    expect(newer).toHaveBeenCalledTimes(1);
  });
});
