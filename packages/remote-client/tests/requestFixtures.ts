import input from "../fixtures/requests.json";
import { isDeepStrictEqual } from "node:util";
import type { RemoteCommand } from "../src/types";

export type ScriptedOutcome = "success" | "transport" | "timeout" | "business" | "authorization";
export interface Expected {
  attemptAtMs: number[]; outcome: "success" | "transport" | "business" | "authorization" | "cancelled";
  recoveries: number; probes: number; completeAtMs: number;
}
export interface RequestCase {
  id: string; kind: "request"; command: RemoteCommand; features: string[];
  outcomes: ScriptedOutcome[]; cancelAtMs: number | null;
  expected: { rust: Expected; typescript: Expected }; difference: string | null;
}
function record(value: unknown, keys: string[]): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key)) || keys.some(key => !(key in value))) throw new Error("unknown/missing request fixture field");
}
function integer(value: unknown): asserts value is number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("invalid request fixture integer");
}
function expectation(value: unknown): asserts value is Expected {
  record(value, ["attemptAtMs", "outcome", "recoveries", "probes", "completeAtMs"]);
  if (!Array.isArray(value.attemptAtMs) || value.attemptAtMs.length === 0) throw new Error("empty request expectation");
  value.attemptAtMs.forEach(integer);
  if (value.attemptAtMs.some((time, index, values) => index > 0 && time < values[index - 1])) throw new Error("unordered request expectation");
  integer(value.recoveries); integer(value.probes); integer(value.completeAtMs);
  if (value.attemptAtMs[value.attemptAtMs.length - 1] > value.completeAtMs) throw new Error("invalid terminal time");
  if (!["success", "transport", "business", "authorization", "cancelled"].includes(String(value.outcome))) throw new Error("unknown expected outcome");
}
record(input, ["version", "suite", "bridgeInstanceId", "requestTimeoutMs", "clockSlackMs", "response", "cases"]);
if (input.version !== 1 || input.suite !== "client.request-policy") throw new Error("unsupported request fixture version/suite");
if (typeof input.bridgeInstanceId !== "string" || input.bridgeInstanceId.length === 0) throw new Error("invalid bridge identity");
integer(input.requestTimeoutMs);
record(input.clockSlackMs, ["rust", "typescript"]);
integer(input.clockSlackMs.rust); integer(input.clockSlackMs.typescript);
if (!Array.isArray(input.cases) || input.cases.length === 0) throw new Error("empty request suite");
const ids = new Set<string>();
for (const scenario of input.cases) {
  record(scenario, ["id", "kind", "command", "features", "outcomes", "cancelAtMs", "expected", "difference"]);
  if (typeof scenario.id !== "string" || !scenario.id.startsWith("client.request-policy.") || ids.has(scenario.id)) throw new Error("invalid/duplicate request case ID");
  ids.add(scenario.id);
  if (scenario.kind !== "request") throw new Error("unsupported request fixture kind");
  if (typeof scenario.command !== "object" || scenario.command === null || !("id" in scenario.command) || !("type" in scenario.command) || typeof scenario.command.id !== "string" || typeof scenario.command.type !== "string") throw new Error("missing command identity");
  if (!Array.isArray(scenario.features) || scenario.features.some(feature => typeof feature !== "string" || feature.length === 0)) throw new Error("invalid capabilities");
  if (!Array.isArray(scenario.outcomes) || scenario.outcomes.length === 0 || scenario.outcomes.some(outcome => !["success", "transport", "timeout", "business", "authorization"].includes(outcome))) throw new Error("unsupported scripted outcome");
  if (scenario.cancelAtMs !== null) integer(scenario.cancelAtMs);
  record(scenario.expected, ["rust", "typescript"]);
  expectation(scenario.expected.rust); expectation(scenario.expected.typescript);
  if (scenario.expected.rust.attemptAtMs.length > scenario.outcomes.length || scenario.expected.typescript.attemptAtMs.length > scenario.outcomes.length) throw new Error("insufficient transport script");
  if (scenario.difference !== null && typeof scenario.difference !== "string") throw new Error("invalid difference note");
  if (!isDeepStrictEqual(scenario.expected.rust, scenario.expected.typescript) && (typeof scenario.difference !== "string" || scenario.difference.trim().length === 0)) throw new Error("undocumented language difference");
}
export const requestSuite = { ...input, cases: input.cases as unknown as RequestCase[] };
