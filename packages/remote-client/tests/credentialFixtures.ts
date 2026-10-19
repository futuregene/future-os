import input from "../fixtures/credentials.json";
export type Action = { type: "close" | "retire" | "supersede" | "release"; target: number };
export type CredentialCase = {
  id: string; kind: "expiry"; expiry: number; malformed: boolean;
  expected: { rust: string; typescript: string }; difference: string | null;
} | {
  id: string; kind: "renewal"; instances: number; actions: Action[];
  expected: { writes: number[]; fresh: boolean[]; continuations: number[]; closed: boolean[] }; difference: string | null;
};
function record(value: unknown, keys: string[]): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key)) || keys.some(key => !(key in value))) throw new Error("unknown/missing credential fixture field");
}
function integer(value: unknown, signed = false): asserts value is number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || (!signed && value < 0)) throw new Error("invalid credential fixture integer");
}
record(input, ["version", "suite", "nowSec", "cases"]);
if (input.version !== 1 || input.suite !== "client.credentials") throw new Error("unsupported credential fixture version/suite");
integer(input.nowSec);
if (input.nowSec === 0 || input.nowSec > Math.floor(Number.MAX_SAFE_INTEGER / 1000)) throw new Error("invalid fixture clock");
if (!Array.isArray(input.cases) || input.cases.length === 0) throw new Error("empty credential suite");
const ids = new Set<string>();
for (const scenario of input.cases) {
  if (typeof scenario.id !== "string" || !scenario.id.startsWith("client.credentials.") || ids.has(scenario.id)) throw new Error("invalid/duplicate credential ID");
  ids.add(scenario.id);
  if (scenario.kind === "expiry") {
    record(scenario, ["id", "kind", "expiry", "malformed", "expected", "difference"]);
    integer(scenario.expiry, true);
    if (typeof scenario.malformed !== "boolean") throw new Error("invalid token fixture flag");
    record(scenario.expected, ["rust", "typescript"]);
    for (const decision of Object.values(scenario.expected)) if (!["refresh", "no_refresh", "invalid"].includes(String(decision))) throw new Error("unsupported policy decision");
    if (scenario.expected.rust !== scenario.expected.typescript && (typeof scenario.difference !== "string" || scenario.difference.trim().length === 0)) throw new Error("undocumented credential policy difference");
  } else if (scenario.kind === "renewal") {
    record(scenario, ["id", "kind", "instances", "actions", "expected", "difference"]);
    integer(scenario.instances);
    if (scenario.instances < 1 || scenario.instances > 4) throw new Error("invalid instance count");
    record(scenario.expected, ["writes", "fresh", "continuations", "closed"]);
    for (const key of ["writes", "fresh", "continuations", "closed"] as const) {
      const vector: unknown = scenario.expected[key];
      if (!Array.isArray(vector) || vector.length !== scenario.instances) throw new Error("invalid expected instance vector");
      for (const value of vector) {
        if (key === "fresh" || key === "closed") { if (typeof value !== "boolean") throw new Error("invalid expected flag"); }
        else integer(value);
      }
    }
    if (!Array.isArray(scenario.actions)) throw new Error("invalid renewal actions");
    const released = new Set<number>();
    for (const action of scenario.actions) {
      record(action, ["type", "target"]); integer(action.target);
      if (action.target >= scenario.instances || !["close", "retire", "supersede", "release"].includes(String(action.type))) throw new Error("unsupported renewal action");
      if (action.type === "release") { if (released.has(action.target)) throw new Error("duplicate host completion"); released.add(action.target); }
    }
    if (released.size !== scenario.instances) throw new Error("every host result must be resolved");
  } else throw new Error("unsupported credential case kind");
  if (scenario.difference !== null && (typeof scenario.difference !== "string" || scenario.difference.trim().length === 0)) throw new Error("invalid ownership difference note");
}
export const credentialSuite = { ...input, cases: input.cases as unknown as CredentialCase[] };
