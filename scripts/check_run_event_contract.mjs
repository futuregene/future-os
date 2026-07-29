import { readFileSync } from "node:fs";

const read = path => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const failures = [];
const requireText = (path, text, description) => {
  if (!read(path).includes(text))
    failures.push(`${path}: missing ${description}`);
};
const forbidText = (path, text, description) => {
  if (read(path).includes(text))
    failures.push(`${path}: contains ${description}`);
};

const proto = "proto/future.proto";
for (const field of ["string session_id = 8;", "int64 epoch = 9;"])
  requireText(proto, field, `canonical envelope field \`${field}\``);
requireText(
  proto,
  "Provider-specific aliases are normalized inside the Agent",
  "provider-normalization contract",
);
forbidText(proto, "Legacy provider-stream aliases", "legacy public-event allowance");

const helper = "agent/src/rpc/prompt_helpers.rs";
for (const mapping of [
  '"toolcall_start" =>',
  'event.event_type = "tool_start"',
  '"toolcall_delta" =>',
  'event.event_type = "tool_delta"',
])
  requireText(helper, mapping, `canonical alias mapping \`${mapping}\``);

const prompt = "agent/src/rpc/session_prompt.rs";
requireText(prompt, "canonical_stream_event(event)", "canonical RPC projection");
if (/move \|event\| \{\s*be\.broadcast/s.test(read(prompt)))
  failures.push(`${prompt}: provider callback bypasses canonical projection`);

for (const path of ["cli/src/rpc/grpc-client.ts", "tui/src/rpc/grpc-client.ts"]) {
  requireText(path, "sessionId: response.sessionId", "sessionId envelope projection");
  requireText(path, "epoch: Number(response.epoch ?? 0)", "epoch envelope projection");
}
for (const path of ["cli/src/rpc/types.ts", "tui/src/rpc/types.ts"]) {
  requireText(path, "requestedRun?: RunTerminalState | null;", "terminal lookup type");
  requireText(path, 'state: "completed" | "error" | "cancelled" | "incomplete"', "terminal state union");
}

if (failures.length) {
  console.error("RunEvent contract drift detected:");
  for (const failure of failures)
    console.error(`  - ${failure}`);
  process.exit(1);
}

console.log("RunEvent contract check passed.");
