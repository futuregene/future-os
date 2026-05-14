#!/usr/bin/env node
/**
 * Test script for gRPC session-related commands.
 * Tests the basic session functionality.
 * 
 * Usage: node test-sessions.cjs [--addr <grpc-addr>]
 */

const grpc = require('@grpc/grpc-js');
const protoLoader = require('@grpc/proto-loader');

const PROTO_PATH = process.env.XIHU_PROTO_PATH ?? "/Users/geilige/xihu/proto/proto/xihu.proto";
const ADDR = process.argv.includes('--addr') 
  ? process.argv[process.argv.indexOf('--addr') + 1] 
  : "localhost:50051";

// Load proto
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
  keepCase: false,
  longs: String,
  enums: String,
  defaults: true,
});
const proto = grpc.loadPackageDefinition(packageDefinition).proto;
const client = new proto.XihuAgent(ADDR, grpc.credentials.createInsecure());

// Helper: execute command
function cmd(type, params = {}) {
  return new Promise((resolve, reject) => {
    const request = { id: String(Date.now()), type, ...params };
    client.ExecuteCommand(request, (err, resp) => {
      if (err) { reject(err); return; }
      if (!resp.success) { reject(new Error(resp.error || "unknown error")); return; }
      try { resolve(resp.data ? JSON.parse(resp.data) : {}); }
      catch { resolve(resp.data); }
    });
  });
}

// Helper: execute command with specific session
function withSession(sessionId, type, params = {}) {
  return cmd(type, { sessionId, ...params });
}

// ANSI colors
const green = (t) => `\x1b[32m${t}\x1b[0m`;
const red = (t) => `\x1b[31m${t}\x1b[0m`;
const yellow = (t) => `\x1b[33m${t}\x1b[0m`;
const dim = (t) => `\x1b[2m${t}\x1b[0m`;

async function run() {
  console.log(yellow(`\n=== gRPC Session Test Suite ===`));
  console.log(dim(`Server: ${ADDR}\n`));

  let passed = 0;
  let failed = 0;

  async function test(name, fn) {
    try {
      process.stdout.write(`Testing: ${name}... `);
      await fn();
      console.log(green("✓ PASS"));
      passed++;
    } catch (err) {
      console.log(red(`✗ FAIL: ${err.message}`));
      failed++;
    }
  }

  // ─── Test 1: Get initial state ───────────────────────────────────────────
  await test("get_state (initial)", async () => {
    const state = await cmd("get_state");
    if (!state.sessionId) throw new Error("No sessionId in state");
    console.log(dim(`    sessionId: ${state.sessionId}`));
  });

  // ─── Test 2: Get available models ───────────────────────────────────────
  await test("get_available_models", async () => {
    const result = await cmd("get_available_models");
    if (!Array.isArray(result.models)) throw new Error("models not array");
    if (result.models.length === 0) throw new Error("no models available");
    console.log(dim(`    count: ${result.models.length}`));
  });

  // ─── Test 3: Create new session ─────────────────────────────────────────
  let newSessionId;
  await test("new_session", async () => {
    const result = await cmd("new_session");
    if (!result.sessionId) throw new Error("No sessionId returned");
    newSessionId = result.sessionId;
    console.log(dim(`    new sessionId: ${newSessionId}`));
  });

  // ─── Test 4: Get state with session_id (verifies session isolation) ─────
  await test("get_state with session_id", async () => {
    const state = await withSession(newSessionId, "get_state");
    if (!state.sessionId) throw new Error("No sessionId in state");
    if (state.sessionId !== newSessionId) {
      throw new Error(`Expected ${newSessionId}, got ${state.sessionId}`);
    }
    console.log(dim(`    sessionId: ${state.sessionId}`));
  });

  // ─── Test 5: Switch session (verifies switch works) ───────────────────
  await test("switch_session", async () => {
    await withSession(newSessionId, "switch_session", { sessionId: newSessionId });
    const state = await withSession(newSessionId, "get_state");
    if (state.sessionId !== newSessionId) {
      throw new Error(`Switch failed, got ${state.sessionId}`);
    }
  });

  // ─── Test 6: Send prompt ────────────────────────────────────────────────
  await test("prompt (streaming)", async () => {
    await withSession(newSessionId, "prompt", { 
      message: "Reply with 'pong'", 
      streamingBehavior: "full" 
    });
    // Wait for streaming to complete
    await new Promise(r => setTimeout(r, 5000));
    const state = await withSession(newSessionId, "get_state");
    if (state.messageCount < 1) {
      throw new Error(`Message not counted: ${state.messageCount}`);
    }
    console.log(dim(`    messageCount: ${state.messageCount}`));
  });

  // ─── Test 7: Set thinking level ─────────────────────────────────────────
  await test("set_thinking_level", async () => {
    await withSession(newSessionId, "set_thinking_level", { level: "low" });
    const state = await withSession(newSessionId, "get_state");
    if (state.thinkingLevel !== "low") {
      throw new Error(`Thinking level not set: ${state.thinkingLevel}`);
    }
  });

  // ─── Test 8: Abort (should work without error) ─────────────────────────
  await test("abort", async () => {
    await withSession(newSessionId, "abort");
  });

  // ─── Summary ────────────────────────────────────────────────────────────
  console.log(yellow(`\n=== Results ===`));
  console.log(green(`Passed: ${passed}`));
  if (failed > 0) {
    console.log(red(`Failed: ${failed}`));
    process.exit(1);
  } else {
    console.log(dim(`All tests passed!\n`));
  }
}

run().catch(err => {
  console.error(red(`\nFatal error: ${err.message}`));
  process.exit(1);
});
