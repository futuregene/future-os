/** Large transport JSON is composed in bounded tasks on web and Hermes.
 * Native JSON.parse handles small values and bounded string fragments. */
export interface JsonDecodeOptions {
  isCurrent?: () => boolean;
  yieldTask?: () => Promise<void>;
  now?: () => number;
  sliceMs?: number;
  nativeLimit?: number;
}
const NATIVE_LIMIT = 32 * 1024;
const STRING_PART = 8 * 1024;
const CHECK_CHARS = 4096;
const BYTE_PART = 64 * 1024;
const MAX_DEPTH = 128;
const MAX_WORK_BYTES = 256 * 1024 * 1024;
const task = () => new Promise<void>(resolve => setTimeout(resolve, 0));
type ArrayFrame = { kind: "array"; value: unknown[]; state: "first" | "value" | "comma" };
type ObjectFrame = { kind: "object"; value: Record<string, unknown>; state: "first" | "key" | "colon" | "value" | "comma"; key?: string };
type Frame = ArrayFrame | ObjectFrame;
let jobs = 0;
let charged = 0;
let waiting = 0;
function check(options: JsonDecodeOptions): void {
  if (options.isCurrent && !options.isCurrent()) throw new Error("stale_json_decode");
}
async function admit(sourceBytes: number, options: JsonDecodeOptions): Promise<() => void> {
  const bytes = Math.max(BYTE_PART, sourceBytes * 8);
  if (!Number.isSafeInteger(bytes) || bytes > MAX_WORK_BYTES) throw new Error("json_decode_budget_exceeded");
  if (waiting >= 32) throw new Error("json_decode_queue_full");
  waiting += 1;
  const deadline = Date.now() + 5000;
  try {
    while (jobs >= 2 || charged + bytes > MAX_WORK_BYTES) {
      check(options);
      if (Date.now() >= deadline) throw new Error("json_decode_admission_timeout");
      await (options.yieldTask ?? task)();
    }
    check(options);
    jobs += 1; charged += bytes;
  } finally { waiting -= 1; }
  return () => { jobs -= 1; charged -= bytes; };
}
function whitespace(char: string | undefined): boolean {
  return char === " " || char === "\n" || char === "\r" || char === "\t";
}
class Composer {
  private at = 0;
  private deadline: number;
  private stack: Frame[] = [];
  private root: unknown;
  private assigned = false;
  private steps = 0;
  private readonly now: () => number;
  private readonly slice: number;
  constructor(private readonly text: string, private readonly options: JsonDecodeOptions) {
    this.now = options.now ?? Date.now;
    this.slice = options.sliceMs ?? 4;
    this.deadline = this.now() + this.slice;
  }
  private *checkpoint(): Generator<void, void, void> {
    check(this.options);
    if (this.now() >= this.deadline) {
      yield;
      check(this.options);
      this.deadline = this.now() + this.slice;
    }
  }
  private invalid(): never { throw new SyntaxError(`Invalid JSON at position ${this.at}`); }
  private *skip(): Generator<void, void, void> {
    let scanned = 0;
    while (whitespace(this.text[this.at])) {
      this.at += 1;
      if (++scanned >= CHECK_CHARS) { scanned = 0; yield* this.checkpoint(); }
    }
  }
  private *string(): Generator<void, string, void> {
    if (this.text[this.at] !== '"') this.invalid();
    this.at += 1;
    let start = this.at;
    let checked = this.at;
    const parts: string[] = [];
    for (;;) {
      const char = this.text[this.at];
      if (char === undefined) this.invalid();
      if (char === '"') {
        parts.push(JSON.parse(`"${this.text.slice(start, this.at)}"`) as string);
        this.at += 1;
        return parts.length === 1 ? parts[0]! : parts.join("");
      }
      // Escapes stay intact. Surrogate halves may cross parts; joining retains
      // exactly the UTF-16 value produced by native JSON.parse.
      if (char === "\\") {
        const escape = this.text[this.at + 1];
        if (escape === undefined) this.invalid();
        this.at += escape === "u" ? 6 : 2;
        if (this.at > this.text.length) this.invalid();
      } else {
        if (char.charCodeAt(0) < 0x20) this.invalid();
        this.at += 1;
      }
      if (this.at - start >= STRING_PART) {
        parts.push(JSON.parse(`"${this.text.slice(start, this.at)}"`) as string);
        start = this.at;
      }
      if (this.at - checked >= CHECK_CHARS) { checked = this.at; yield* this.checkpoint(); }
    }
  }
  private accept(value: unknown): void {
    const parent = this.stack[this.stack.length - 1];
    if (!parent) {
      if (this.assigned) this.invalid();
      this.root = value; this.assigned = true;
    } else if (parent.kind === "array") {
      if (parent.state !== "first" && parent.state !== "value") this.invalid();
      parent.value.push(value); parent.state = "comma";
    } else {
      if (parent.state !== "value" || parent.key === undefined) this.invalid();
      Object.defineProperty(parent.value, parent.key, { value, writable: true, enumerable: true, configurable: true });
      parent.key = undefined; parent.state = "comma";
    }
  }
  private *value(): Generator<void, void, void> {
    const char = this.text[this.at];
    if (char === "{" || char === "[") {
      const frame: Frame = char === "{" ? { kind: "object", value: {}, state: "first" }
        : { kind: "array", value: [], state: "first" };
      this.accept(frame.value); this.stack.push(frame);
      if (this.stack.length > MAX_DEPTH) throw new Error("json_decode_depth_exceeded");
      this.at += 1;
    } else if (char === '"') {
      this.accept(yield* this.string());
    } else {
      const start = this.at;
      while (this.at < this.text.length && !whitespace(this.text[this.at]) && !",]}:".includes(this.text[this.at]!)) {
        this.at += 1;
        // Supported producer numbers are finite serialized values; do not
        // hand a pathological literal to an unbounded native parsing phase.
        if (this.at - start > STRING_PART) throw new Error("json_decode_literal_too_large");
      }
      if (this.at === start) this.invalid();
      this.accept(JSON.parse(this.text.slice(start, this.at)) as unknown);
    }
  }
  *compose(): Generator<void, unknown, void> {
    for (;;) {
      if (++this.steps % 256 === 0) yield* this.checkpoint();
      yield* this.skip();
      const frame = this.stack[this.stack.length - 1];
      if (!frame) {
        if (this.assigned) {
          if (this.at !== this.text.length) this.invalid();
          check(this.options); return this.root;
        }
        yield* this.value(); continue;
      }
      const char = this.text[this.at];
      if (frame.kind === "array") {
        if (char === "]" && (frame.state === "first" || frame.state === "comma")) {
          this.stack.pop(); this.at += 1;
        } else if (frame.state === "comma") {
          if (char !== ",") this.invalid();
          this.at += 1; frame.state = "value";
        } else { yield* this.value(); }
      } else if (frame.state === "first" || frame.state === "key") {
        if (char === "}" && frame.state === "first") { this.stack.pop(); this.at += 1; }
        else { frame.key = yield* this.string(); frame.state = "colon"; }
      } else if (frame.state === "colon") {
        if (char !== ":") this.invalid();
        this.at += 1; frame.state = "value";
      } else if (frame.state === "comma") {
        if (char === "}") { this.stack.pop(); this.at += 1; }
        else { if (char !== ",") this.invalid(); this.at += 1; frame.state = "key"; }
      } else { yield* this.value(); }
    }
  }
}
async function parse<T>(text: string, options: JsonDecodeOptions): Promise<T> {
  check(options);
  if (text.length <= (options.nativeLimit ?? NATIVE_LIMIT)) {
    const value = JSON.parse(text) as T; check(options); return value;
  }
  const parser = new Composer(text, options).compose();
  for (;;) {
    const step = parser.next();
    if (step.done) return step.value as T;
    await (options.yieldTask ?? task)(); check(options);
  }
}
export async function parseJsonText<T>(text: string, options: JsonDecodeOptions = {}): Promise<T> {
  check(options);
  if (text.length <= (options.nativeLimit ?? NATIVE_LIMIT)) return parse<T>(text, options);
  const release = await admit(text.length * 2, options);
  try { return await parse<T>(text, options); } finally { release(); }
}
export async function decodeJsonBytes<T>(bytes: Uint8Array, options: JsonDecodeOptions = {}): Promise<T> {
  check(options);
  if (bytes.length <= (options.nativeLimit ?? NATIVE_LIMIT)) return parse<T>(new TextDecoder().decode(bytes), options);
  const release = await admit(bytes.length, options);
  try {
    const decoder = new TextDecoder();
    const parts: string[] = [];
    const now = options.now ?? Date.now;
    let deadline = now() + (options.sliceMs ?? 4);
    for (let offset = 0; offset < bytes.length; offset += BYTE_PART) {
      check(options);
      parts.push(decoder.decode(bytes.subarray(offset, offset + BYTE_PART), { stream: true }));
      if (now() >= deadline) { await (options.yieldTask ?? task)(); check(options); deadline = now() + (options.sliceMs ?? 4); }
    }
    parts.push(decoder.decode());
    await (options.yieldTask ?? task)(); check(options);
    const text = parts.join(""); parts.length = 0;
    return await parse<T>(text, options);
  } finally { release(); }
}
