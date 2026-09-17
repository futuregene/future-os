// @vitest-environment jsdom
import { Terminal } from "@xterm/xterm";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { terminalKeyPolicy } from "./keyPolicy";
import { installMacInputWorkaround } from "./macInput";

// xterm can use its DOM renderer without a canvas in these keyboard-only tests.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = vi.fn(() => null);
});

const WEBKIT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
let terminal: Terminal;
let host: HTMLDivElement;
let sent: string[];
let removeWorkaround: (() => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(WEBKIT);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addListener() {}, removeListener() {} }));
  vi.stubGlobal("IntersectionObserver", class {
    observe() {}
    disconnect() {}
  });
  host = document.createElement("div");
  document.body.append(host);
  terminal = new Terminal();
  terminal.open(host);
  terminal.attachCustomKeyEventHandler(event => terminalKeyPolicy(event, true));
  sent = [];
  terminal.onData(data => sent.push(data));
});

afterEach(() => {
  removeWorkaround?.();
  removeWorkaround = undefined;
  terminal?.dispose();
  host?.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.clearAllTimers();
  vi.useRealTimers();
});

function key(type: string, value: string, keyCode: number, extra: KeyboardEventInit = {}) {
  terminal.textarea!.dispatchEvent(new KeyboardEvent(type, {
    key: value,
    keyCode,
    bubbles: true,
    cancelable: true,
    ...extra,
  }));
}

function text(data: string, extra: InputEventInit = {}) {
  for (const type of ["beforeinput", "input"]) {
    if (type === "input") {
      terminal.textarea!.value += data;
    }
    terminal.textarea!.dispatchEvent(new InputEvent(type, {
      data,
      inputType: "insertText",
      composed: true,
      bubbles: true,
      cancelable: true,
      ...extra,
    }));
  }
}

/** WebKit IME-source ASCII: committed text precedes its keydown, no keypress. */
function webkitKey(value: string) {
  text(value);
  key("keydown", value, 229);
  // Separate native keystrokes are event-loop turns, even while keys overlap.
  vi.advanceTimersByTime(10);
}

describe("macOS WebKit input ordering (real xterm)", () => {
  it("reproduces the upstream loss when a previous key has not been released", () => {
    webkitKey("h");
    key("keyup", "h", 72);
    webkitKey("t");
    webkitKey("o");
    key("keyup", "t", 84);
    webkitKey("p");
    vi.advanceTimersByTime(10);
    expect(sent.join("")).toBe("htp");
  });

  it("sends every overlapping htop keystroke exactly once", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    for (const value of "htop") {
      webkitKey(value);
    }
    for (const value of "htop") {
      key("keyup", value, value.toUpperCase().charCodeAt(0));
    }
    vi.advanceTimersByTime(10);
    expect(sent.join("")).toBe("htop");
  });

  it("preserves the first shifted and Caps Lock character", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "Shift", 16, { shiftKey: true });
    text("#");
    key("keydown", "#", 229, { shiftKey: true });
    key("keyup", "#", 51, { shiftKey: true });
    key("keyup", "Shift", 16);
    vi.advanceTimersByTime(10);
    key("keydown", "CapsLock", 20);
    text("A");
    key("keydown", "A", 65);
    vi.advanceTimersByTime(10);
    expect(sent.join("")).toBe("#A");
  });

  it("keeps keypress deduplication for conventional uppercase input", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "A", 65, { shiftKey: true });
    key("keypress", "A", 65, { shiftKey: true, charCode: 65 });
    text("A");
    key("keyup", "A", 65);
    expect(sent.join("")).toBe("A");
  });

  it("leaves ordinary keydown input and control keys to xterm", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    for (const value of "htop") {
      key("keydown", value, value.toUpperCase().charCodeAt(0));
    }
    key("keydown", "Backspace", 8);
    key("keydown", "Enter", 13);
    key("keydown", "c", 67, { ctrlKey: true });
    expect(sent.join("")).toBe("htop\x7F\r\x03");
  });

  it.each([
    { isComposing: true },
    { inputType: "insertCompositionText" },
    { inputType: "insertFromComposition" },
    { inputType: "insertFromPaste" },
    { composed: false },
    { data: null },
  ])("does not reset xterm's guard for non-committed text: %j", (extra) => {
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "Shift", 16);
    terminal.textarea!.dispatchEvent(new InputEvent("beforeinput", {
      data: "中",
      inputType: "insertText",
      composed: true,
      ...extra,
    }));
    // A subsequent input with no qualifying beforeinput must still be blocked.
    terminal.textarea!.dispatchEvent(new InputEvent("input", {
      data: "x",
      inputType: "insertText",
      composed: true,
    }));
    expect(sent).toEqual([]);
  });

  it("commits Chinese composition once without leaking composing Backspace", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    const textarea = terminal.textarea!;
    textarea.dispatchEvent(new CompositionEvent("compositionstart"));
    textarea.dispatchEvent(new CompositionEvent("compositionupdate", { data: "你好" }));
    text("你好", { inputType: "insertCompositionText", isComposing: true });
    key("keydown", "Backspace", 0, { isComposing: true });
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    textarea.dispatchEvent(new CompositionEvent("compositionend", { data: "你好" }));
    vi.advanceTimersByTime(10);
    expect(sent.join("")).toBe("你好");
    key("keydown", "Enter", 13);
    expect(sent.join("")).toBe("你好\r");
  });

  it("still releases the panel shortcut without sending it to the shell", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "j", 74, { metaKey: true });
    expect(sent).toEqual([]);
  });

  it("does not interfere with screen reader mode", () => {
    terminal.options.screenReaderMode = true;
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "Shift", 16);
    text("#");
    expect(sent).toEqual([]);
  });

  it.each([
    `${WEBKIT} Chrome/140.0.0.0 Safari/537.36`,
    "Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Edg/140.0.0.0",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15) Gecko/20100101 Firefox/140.0",
  ])("is disabled outside macOS WebKit: %s", (ua) => {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);
    removeWorkaround = installMacInputWorkaround(terminal);
    key("keydown", "Shift", 16);
    text("#");
    expect(sent).toEqual([]);
  });

  it("removes the listener on disposal", () => {
    removeWorkaround = installMacInputWorkaround(terminal);
    removeWorkaround();
    key("keydown", "Shift", 16);
    text("#");
    expect(sent).toEqual([]);
  });
});
