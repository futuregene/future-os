/**
 * StdinBuffer buffers stdin input and emits complete escape sequences.
 * Without buffering, partial sequences (e.g. mouse SGR split across chunks)
 * can be misinterpreted as regular keypresses.
 *
 * Based on code from OpenTUI (https://github.com/anomalyco/opentui)
 * MIT License - Copyright (c) 2025 opentui
 */

import { EventEmitter } from "events";

const ESC = "\x1b";
const BRACKETED_PASTE_START = "\x1b[200~";
const BRACKETED_PASTE_END = "\x1b[201~";

function isCompleteSequence(data: string): "complete" | "incomplete" | "not-escape" {
  if (!data.startsWith(ESC)) return "not-escape";
  if (data.length === 1) return "incomplete";

  const afterEsc = data.slice(1);

  // CSI sequences: ESC [
  if (afterEsc.startsWith("[")) {
    if (afterEsc.startsWith("[M")) {
      return data.length >= 6 ? "complete" : "incomplete";
    }
    return isCompleteCsiSequence(data);
  }

  // OSC sequences: ESC ]
  if (afterEsc.startsWith("]")) {
    return isCompleteOscSequence(data);
  }

  // DCS sequences: ESC P ... ESC \
  if (afterEsc.startsWith("P")) {
    return isCompleteDcsSequence(data);
  }

  // APC sequences: ESC _ ... ESC \ (Kitty graphics)
  if (afterEsc.startsWith("_")) {
    return isCompleteApcSequence(data);
  }

  // SS3 sequences: ESC O
  if (afterEsc.startsWith("O")) {
    return afterEsc.length >= 2 ? "complete" : "incomplete";
  }

  // Meta key: ESC + single character
  if (afterEsc.length === 1) return "complete";

  return "complete";
}

function isCompleteCsiSequence(data: string): "complete" | "incomplete" {
  if (!data.startsWith(`${ESC}[`)) return "complete";
  if (data.length < 3) return "incomplete";

  const payload = data.slice(2);
  const lastChar = payload[payload.length - 1];
  const lastCharCode = lastChar.charCodeAt(0);

  if (lastCharCode >= 0x40 && lastCharCode <= 0x7e) {
    // SGR mouse: ESC[<B;X;Ym or ESC[<B;X;YM
    if (payload.startsWith("<")) {
      const mouseMatch = /^<\d+;\d+;\d+[Mm]$/.test(payload);
      if (mouseMatch) return "complete";
      if (lastChar === "M" || lastChar === "m") {
        const parts = payload.slice(1, -1).split(";");
        if (parts.length === 3 && parts.every((p) => /^\d+$/.test(p))) {
          return "complete";
        }
      }
      return "incomplete";
    }
    return "complete";
  }
  return "incomplete";
}

function isCompleteOscSequence(data: string): "complete" | "incomplete" {
  if (!data.startsWith(`${ESC}]`)) return "complete";
  if (data.endsWith(`${ESC}\\`) || data.endsWith("\x07")) return "complete";
  return "incomplete";
}

function isCompleteDcsSequence(data: string): "complete" | "incomplete" {
  if (!data.startsWith(`${ESC}P`)) return "complete";
  if (data.endsWith(`${ESC}\\`)) return "complete";
  return "incomplete";
}

function isCompleteApcSequence(data: string): "complete" | "incomplete" {
  if (!data.startsWith(`${ESC}_`)) return "complete";
  if (data.endsWith(`${ESC}\\`)) return "complete";
  return "incomplete";
}

function extractCompleteSequences(buffer: string): { sequences: string[]; remainder: string } {
  const sequences: string[] = [];
  let pos = 0;

  while (pos < buffer.length) {
    const remaining = buffer.slice(pos);
    if (remaining.startsWith(ESC)) {
      let seqEnd = 1;
      while (seqEnd <= remaining.length) {
        const candidate = remaining.slice(0, seqEnd);
        const status = isCompleteSequence(candidate);
        if (status === "complete") {
          sequences.push(candidate);
          pos += seqEnd;
          break;
        } else if (status === "incomplete") {
          seqEnd++;
        } else {
          sequences.push(candidate);
          pos += seqEnd;
          break;
        }
      }
      if (seqEnd > remaining.length) {
        return { sequences, remainder: remaining };
      }
    } else {
      sequences.push(remaining[0]!);
      pos++;
    }
  }

  return { sequences, remainder: "" };
}

export type StdinBufferOptions = {
  timeout?: number;
};

export type StdinBufferEventMap = {
  data: [string];
  paste: [string];
};

export class StdinBuffer extends EventEmitter<StdinBufferEventMap> {
  private buffer = "";
  private timeout: ReturnType<typeof setTimeout> | null = null;
  private readonly timeoutMs: number;
  private pasteMode = false;
  private pasteBuffer = "";

  constructor(options: StdinBufferOptions = {}) {
    super();
    this.timeoutMs = options.timeout ?? 10;
  }

  process(data: string | Buffer): void {
    if (this.timeout) {
      clearTimeout(this.timeout);
      this.timeout = null;
    }

    let str: string;
    if (Buffer.isBuffer(data)) {
      if (data.length === 1 && data[0]! > 127) {
        const byte = data[0]! - 128;
        str = `\x1b${String.fromCharCode(byte)}`;
      } else {
        str = data.toString();
      }
    } else {
      str = data;
    }

    if (str.length === 0 && this.buffer.length === 0) {
      this.emit("data", "");
      return;
    }

    this.buffer += str;

    if (this.pasteMode) {
      this.pasteBuffer += this.buffer;
      this.buffer = "";
      const endIndex = this.pasteBuffer.indexOf(BRACKETED_PASTE_END);
      if (endIndex !== -1) {
        const pastedContent = this.pasteBuffer.slice(0, endIndex);
        const remaining = this.pasteBuffer.slice(endIndex + BRACKETED_PASTE_END.length);
        this.pasteMode = false;
        this.pasteBuffer = "";
        this.emit("paste", pastedContent);
        if (remaining.length > 0) this.process(remaining);
      }
      return;
    }

    const startIndex = this.buffer.indexOf(BRACKETED_PASTE_START);
    if (startIndex !== -1) {
      if (startIndex > 0) {
        const beforePaste = this.buffer.slice(0, startIndex);
        const result = extractCompleteSequences(beforePaste);
        for (const sequence of result.sequences) {
          this.emit("data", sequence);
        }
      }
      this.buffer = this.buffer.slice(startIndex + BRACKETED_PASTE_START.length);
      this.pasteMode = true;
      this.pasteBuffer = this.buffer;
      this.buffer = "";

      const endIndex = this.pasteBuffer.indexOf(BRACKETED_PASTE_END);
      if (endIndex !== -1) {
        const pastedContent = this.pasteBuffer.slice(0, endIndex);
        const remaining = this.pasteBuffer.slice(endIndex + BRACKETED_PASTE_END.length);
        this.pasteMode = false;
        this.pasteBuffer = "";
        this.emit("paste", pastedContent);
        if (remaining.length > 0) this.process(remaining);
      }
      return;
    }

    const result = extractCompleteSequences(this.buffer);
    this.buffer = result.remainder;

    for (const sequence of result.sequences) {
      this.emit("data", sequence);
    }

    if (this.buffer.length > 0) {
      this.timeout = setTimeout(() => {
        const flushed = this.flush();
        for (const sequence of flushed) {
          this.emit("data", sequence);
        }
      }, this.timeoutMs);
    }
  }

  flush(): string[] {
    if (this.timeout) {
      clearTimeout(this.timeout);
      this.timeout = null;
    }
    if (this.buffer.length === 0) return [];
    const sequences = [this.buffer];
    this.buffer = "";
    return sequences;
  }

  clear(): void {
    if (this.timeout) {
      clearTimeout(this.timeout);
      this.timeout = null;
    }
    this.buffer = "";
    this.pasteMode = false;
    this.pasteBuffer = "";
  }

  getBuffer(): string {
    return this.buffer;
  }

  destroy(): void {
    this.clear();
  }
}
