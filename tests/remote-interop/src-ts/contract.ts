import input from "../../../packages/remote-client/fixtures/client.json";
import isolationInput from "../../../packages/remote-client/fixtures/isolation.json";
import type { RemoteCommand } from "@future-os/remote-client/types";

export interface Step { command: RemoteCommand & { id: string }; data?: unknown; error?: string }
export interface Case { id: string; kind: "request"; steps: Step[]; expectedExecutions: number }
function record(value: unknown, allowed: string[], required: string[]): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("invalid shared fixture object");
  const keys = Object.keys(value);
  if (keys.some(key => !allowed.includes(key)) || required.some(key => !keys.includes(key))) throw new Error("unknown/missing shared fixture field");
}
record(input, ["version", "fileBytes", "event", "cases", "flowIds"], ["version", "fileBytes", "event", "cases", "flowIds"]);
if (input.version !== 1) throw new Error("unsupported Client fixture version");
if (!Array.isArray(input.fileBytes) || input.fileBytes.length === 0 || input.fileBytes.some(byte => !Number.isInteger(byte) || byte < 0 || byte > 255)) throw new Error("invalid fixture bytes");
record(input.event, ["sessionId", "type", "data", "runId", "idx", "epoch", "eventId", "timestamp", "sessionIdx", "runSequence"], ["sessionId", "type", "data", "runId", "idx", "epoch", "eventId", "timestamp", "sessionIdx", "runSequence"]);
for (const key of ["sessionId", "type", "data", "runId", "eventId", "timestamp"] as const) {
  if (typeof input.event[key] !== "string") throw new Error("invalid shared event text");
}
for (const key of ["idx", "epoch", "sessionIdx", "runSequence"] as const) {
  if (!Number.isSafeInteger(input.event[key])) throw new Error("invalid shared event cursor");
}
if (!Array.isArray(input.cases)) throw new Error("invalid Client fixture cases");
const ids = new Set<string>();
record(input.flowIds, ["firstPair", "event", "transfer", "reconnect", "shutdown"], ["firstPair", "event", "transfer", "reconnect", "shutdown"]);
for (const id of Object.values(input.flowIds)) {
  if (typeof id !== "string" || !id.startsWith("client.") || ids.has(id)) throw new Error("invalid/duplicate Client flow ID");
  ids.add(id);
}
for (const item of input.cases) {
  record(item, ["id", "kind", "steps", "expectedExecutions"], ["id", "kind", "steps", "expectedExecutions"]);
  if (typeof item.id !== "string" || !item.id.startsWith("client.") || ids.has(item.id)) throw new Error("invalid/duplicate Client case ID");
  ids.add(item.id);
  if (item.kind !== "request") throw new Error("unsupported Client case kind");
  if (!Number.isSafeInteger(item.expectedExecutions) || item.expectedExecutions < 0 || !Array.isArray(item.steps) || item.steps.length === 0) throw new Error("invalid Client expectation");
  let commandId: string | undefined;
  for (const step of item.steps) {
    record(step, ["command", "data", "error"], ["command"]);
    if (("data" in step) === ("error" in step)) throw new Error("ambiguous step expectation");
    if (step.error !== undefined && typeof step.error !== "string") throw new Error("invalid expected error");
    if (typeof step.command !== "object" || step.command === null || !("id" in step.command) || !("type" in step.command) ||
      typeof step.command.id !== "string" || typeof step.command.type !== "string") throw new Error("invalid command identity");
    commandId ??= step.command.id;
    if (step.command.id !== commandId) throw new Error("inconsistent request identity");
  }
}
export const suite = { ...input, cases: input.cases as unknown as Case[] };

record(isolationInput, ["version", "suite", "endpoints", "requestId", "flowIds"], ["version", "suite", "endpoints", "requestId", "flowIds"]);
if (isolationInput.version !== 1 || isolationInput.suite !== "client.isolation") throw new Error("unsupported isolation suite");
if (!Array.isArray(isolationInput.endpoints) || isolationInput.endpoints.length !== 2 || typeof isolationInput.requestId !== "string" || !isolationInput.requestId) throw new Error("invalid isolation inputs");
const labels = new Set<string>();
for (const endpoint of isolationInput.endpoints) {
  record(endpoint, ["label", "message"], ["label", "message"]);
  if (typeof endpoint.label !== "string" || !/^[A-Za-z0-9_]{1,32}$/.test(endpoint.label) || labels.has(endpoint.label) || typeof endpoint.message !== "string" || !endpoint.message) throw new Error("invalid/duplicate endpoint");
  labels.add(endpoint.label);
}
record(isolationInput.flowIds, ["requests", "catalog", "events", "disconnect", "reconnect"], ["requests", "catalog", "events", "disconnect", "reconnect"]);
const isolationIds = new Set<string>();
for (const id of Object.values(isolationInput.flowIds)) {
  if (typeof id !== "string" || !id.startsWith("client.isolation.") || isolationIds.has(id)) throw new Error("invalid/duplicate isolation ID");
  isolationIds.add(id);
}
export const isolation = isolationInput;
