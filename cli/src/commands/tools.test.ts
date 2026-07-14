/**
 * Tests for parseToolArgs — covers Windows cmd.exe quote-stripping recovery,
 * split-arg joining, nested JSON, and depth-aware comma splitting.
 *
 * Run: npx tsx src/commands/tools.test.ts
 */

import { parseToolArgs } from "./tools.js";

let passed = 0;
let failed = 0;

function test(name: string, fn: () => void) {
  try {
    fn();
    passed++;
  } catch (e) {
    failed++;
    console.error(`FAIL: ${name}`);
    console.error(`  ${e instanceof Error ? e.message : String(e)}`);
  }
}

function assert(condition: boolean, msg: string) {
  if (!condition) throw new Error(msg);
}

function deepEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

// ─── Normal JSON (should always work) ────────────────────────────────────

test("simple valid JSON", () => {
  const result = parseToolArgs('{"prompt":"hello"}');
  assert(deepEqual(result, { prompt: "hello" }), "expected prompt:hello");
});

test("multiple keys", () => {
  const result = parseToolArgs('{"prompt":"hello","size":"1024x1024"}');
  assert(
    deepEqual(result, { prompt: "hello", size: "1024x1024" }),
    "expected two keys",
  );
});

test("numeric and boolean values", () => {
  const result = parseToolArgs('{"n":5,"flag":true,"ratio":3.14}');
  assert(deepEqual(result, { n: 5, flag: true, ratio: 3.14 }), "expected typed values");
});

// ─── Single-quote wrapped JSON (docs recommend this pattern) ────────────

test("single-quote wrapped JSON", () => {
  const result = parseToolArgs(`'{"prompt":"hello"}'`);
  assert(deepEqual(result, { prompt: "hello" }), "expected prompt:hello");
});

// ─── cmd.exe stripped quotes — parseCmdObject recovery ──────────────────

test("cmd.exe stripped quotes, no spaces", () => {
  // cmd.exe turns {"prompt":"hello"} into {prompt:hello}
  const result = parseToolArgs("{prompt:hello}");
  assert(deepEqual(result, { prompt: "hello" }), "expected prompt:hello");
});

test("cmd.exe stripped quotes with single-quote wrapping", () => {
  // cmd.exe strips inner " but leaves outer '
  const result = parseToolArgs("'{prompt:hello,size:1024x1024}'");
  assert(
    deepEqual(result, { prompt: "hello", size: "1024x1024" }),
    "expected two keys",
  );
});

test("cmd.exe stripped quotes, value with spaces (rejoined args)", () => {
  // Simulates args rejoined after cmd.exe split: '{prompt:a beautiful fox,size:1024x1024}'
  const result = parseToolArgs("{prompt:a beautiful fox,size:1024x1024}");
  assert(
    deepEqual(result, { prompt: "a beautiful fox", size: "1024x1024" }),
    "expected prompt with spaces",
  );
});

test("cmd.exe stripped quotes, value with leading/trailing single quotes", () => {
  // Some shells pass through stray quotes
  const result = parseToolArgs("'{prompt:a beautiful fox}'");
  assert(
    deepEqual(result, { prompt: "a beautiful fox" }),
    "expected prompt with spaces after strip",
  );
});

// ─── Value containing commas ────────────────────────────────────────────
// NOTE: When cmd.exe strips ALL double quotes, commas inside string values
// become indistinguishable from field-separator commas in flat JSON. These
// cases are handled by the agent-side rewrite (sandbox/mod.rs) which pipes
// JSON through a temp file and --stdin, bypassing cmd.exe entirely.

test("valid JSON with comma in value (via --stdin / agent rewrite)", () => {
  // The agent-side rewrite preserves JSON intact when piping through stdin
  const result = parseToolArgs('{"prompt":"hello, world"}');
  assert(
    deepEqual(result, { prompt: "hello, world" }),
    "valid JSON with comma should work",
  );
});

test("valid JSON with commas in multiple values", () => {
  const result = parseToolArgs('{"prompt":"hello, world","style":"calm, serene"}');
  assert(
    deepEqual(result, { prompt: "hello, world", style: "calm, serene" }),
    "multiple comma-values should work",
  );
});

// ─── Nested objects (depth-aware splitting) ─────────────────────────────

test("nested object value", () => {
  // {"messages":[{"role":"user","content":"hello"}]}
  const result = parseToolArgs("{messages:[{role:user,content:hello}]}");
  assert(result && typeof result === "object", "expected object");
  assert(Array.isArray(result.messages), "messages should be array");
  const arr = result.messages as Array<Record<string, unknown>>;
  assert(deepEqual(arr[0], { role: "user", content: "hello" }), "expected nested object");
});

test("multiple nested fields with comma", () => {
  const result = parseToolArgs(
    "{messages:[{role:user,content:hello world}],model:gpt-4}",
  );
  assert(result && typeof result === "object", "expected object");
  assert(Array.isArray(result.messages), "messages should be array");
  assert(result.model === "gpt-4", "expected model field");
});

// ─── PowerShell backslash-escaped quotes ─────────────────────────────────

test("PowerShell backslash-escaped JSON", () => {
  // PowerShell can pass: {\"prompt\":\"hello\"}
  const result = parseToolArgs('{\\"prompt\\":\\"hello\\"}');
  assert(deepEqual(result, { prompt: "hello" }), "expected prompt:hello");
});

// ─── Edge cases ──────────────────────────────────────────────────────────

test("empty object", () => {
  const result = parseToolArgs("{}");
  assert(deepEqual(result, {}), "expected empty object");
});

test("null value", () => {
  const result = parseToolArgs("{key:null}");
  assert(result.key === null, "expected null value");
});

test("boolean false value", () => {
  const result = parseToolArgs("{flag:false}");
  assert(result.flag === false, "expected false");
});

test("error includes helpful message", () => {
  try {
    parseToolArgs("not json at all");
    throw new Error("should have thrown");
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    assert(
      msg.includes("--args must be a JSON object"),
      "expected helpful error message",
    );
  }
});

// ─── End-to-end: simulate rejoined argv after cmd.exe split ──────────────

// Simulates what the CLI's tools() function does: joining split argv
// elements back together after cmd.exe split them on spaces.
function simulateRejoinedArgs(argv: string[]): Record<string, unknown> {
  const argsIdx = argv.indexOf("--args");
  if (argsIdx === -1 || argsIdx + 1 >= argv.length) {
    throw new Error("--args not found");
  }
  let raw = argv[argsIdx + 1];
  for (let i = argsIdx + 2; i < argv.length && !argv[i].startsWith("--"); i++) {
    raw += " " + argv[i];
  }
  return parseToolArgs(raw);
}

test("e2e: cmd.exe split on prompt spaces (most common failure)", () => {
  // Model wrote: --args '{"prompt":"a beautiful fox","size":"1024x1024"}'
  // cmd.exe stripped " → argv: ['--args', ''{prompt:a', 'beautiful', 'fox,size:1024x1024}'']
  const result = simulateRejoinedArgs([
    "--args",
    "'{prompt:a",
    "beautiful",
    "fox,size:1024x1024}'",
  ]);
  assert(
    deepEqual(result, { prompt: "a beautiful fox", size: "1024x1024" }),
    "expected prompt with spaces rejoined",
  );
});

test("e2e: cmd.exe split on multiple space-containing values", () => {
  // Model wrote: --args '{"prompt":"ancient oak tree","style":"oil painting"}'
  const result = simulateRejoinedArgs([
    "--args",
    "'{prompt:ancient",
    "oak",
    "tree,style:oil",
    "painting}'",
  ]);
  assert(
    deepEqual(result, { prompt: "ancient oak tree", style: "oil painting" }),
    "expected two fields with spaces",
  );
});

test("e2e: flag after --args is not joined", () => {
  // Model wrote: --args '{"prompt":"hello"}' --output result.png
  const result = simulateRejoinedArgs([
    "--args",
    "'{prompt:hello}'",
    "--output",
    "result.png",
  ]);
  assert(
    deepEqual(result, { prompt: "hello" }),
    "expected only args JSON, not the next flag",
  );
});

// ─── Summary ─────────────────────────────────────────────────────────────

console.log(`\n${passed} passed, ${failed} failed, ${passed + failed} total`);
if (failed > 0) process.exit(1);
