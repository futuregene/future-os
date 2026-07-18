/**
 * `future session` command — list, inspect, rename, and delete agent sessions.
 */

import { RunClient } from "../rpc/grpc-client.js";

const grpcAddr = () => process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";

function help(): void {
  console.log(`future session — manage agent sessions

Usage:
  future session list [--json]                       List all sessions
  future session info <id>                           Show session details + stats
  future session rename <id> <name>                  Give a session a readable name
  future session delete <id>                         Delete a session

Session data is stored at ~/.future/agent/sessions/`);
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1) + "…";
}

function humanTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
  return String(n);
}

function ago(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

// ─── List ─────────────────────────────────────────────────────────────────

async function listSessions(jsonFlag: boolean): Promise<void> {
  const client = new RunClient(grpcAddr());
  const { sessions } = await client.listSessions();

  if (jsonFlag) {
    console.log(JSON.stringify({ sessions }, null, 2));
    return;
  }

  if (sessions.length === 0) {
    console.log("No sessions found.");
    return;
  }

  sessions.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());

  // Header
  console.log(`  ${"SESSION ID".padEnd(24)} ${"TITLE".padEnd(38)} ${"UPDATED".padEnd(10)} ${"MODEL".padEnd(28)} QUERIES`);
  console.log(`  ${"—".repeat(24)} ${"—".repeat(38)} ${"—".repeat(10)} ${"—".repeat(28)} ———————`);

  for (const s of sessions) {
    const title = s.session_name || s.first_message
      ? truncate(s.session_name || s.first_message || "", 42)
      : "(untitled)";
    const model = s.model
      ? s.model.length > 28 ? s.model.slice(0, 27) + "…" : s.model
      : "—";
    const q = s.query_count ? `${s.query_count}` : "—";
    console.log(`  ${s.id.padEnd(24)} ${title.padEnd(38)} ${ago(s.updated_at).padEnd(10)} ${model.padEnd(28)} ${q}`);
  }
  console.log(`\n${sessions.length} sessions.`);
}

// ─── Info ─────────────────────────────────────────────────────────────────

async function info(sessionId: string): Promise<void> {
  const client = new RunClient(grpcAddr());
  const data = await client.getSessionEntries(sessionId);

  if (!data.entries || data.entries.length === 0) {
    console.error(`Session not found: ${sessionId}`);
    process.exit(1);
  }

  // Metadata from the session_info entry
  const infoEntry = data.entries.find(e => e.role === "system") as Record<string, unknown> | undefined;
  const content = (infoEntry?.content ?? {}) as Record<string, unknown>;
  const model = (infoEntry?.model as string) || (content?.model as string) || "?";
  const thinkingLevel = (infoEntry?.thinking_level as string) || (content?.thinking_level as string) || "?";
  const sessionName = (content?.session_name as string) || "(untitled)";
  const cwd = (content?.cwd as string) || "";

  // Count entries by role/type
  const roles = new Map<string, number>();
  let toolCalls = 0;
  for (const e of data.entries) {
    const t = (e.type as string) || (e.role as string) || "?";
    roles.set(t, (roles.get(t) ?? 0) + 1);
    if (e.tool_calls && Array.isArray(e.tool_calls)) toolCalls += (e.tool_calls as unknown[]).length;
  }

  const users = roles.get("user") ?? 0;
  const assistants = roles.get("assistant") ?? 0;
  const tools = roles.get("tool") ?? 0;
  const system = roles.get("session_info") ?? roles.get("system") ?? 0;
  // Compaction entries are labeled "compacted"
  const compacted = roles.get("compaction") ?? 0;

  console.log(`Session:  ${sessionId}`);
  console.log(`  Name:        ${sessionName}`);
  console.log(`  Model:       ${model}`);
  console.log(`  Thinking:    ${thinkingLevel}`);
  if (cwd) console.log(`  CWD:         ${cwd}`);

  // Token/cost from session_info content (persisted per-run)
  const tokensIn = Number(content?.tokens_in ?? 0);
  const tokensOut = Number(content?.tokens_out ?? 0);
  const tokensCacheR = Number(content?.tokens_cache_r ?? 0);
  const tokensCacheW = Number(content?.tokens_cache_w ?? 0);
  const totalCost = Number(content?.total_cost ?? 0);

  console.log(`  Messages:    ${data.entries.length} (${users} user, ${assistants} assistant, ${tools} tool${system ? `, ${system} system` : ""}${compacted ? `, ${compacted} compacted` : ""})`);
  console.log(`  Tool calls:  ${toolCalls}`);
  if (tokensIn + tokensOut > 0) {
    console.log(`  Tokens:      in=${humanTokens(tokensIn)} out=${humanTokens(tokensOut)}`);
    if (tokensCacheR + tokensCacheW > 0) console.log(`  Cache:       r=${humanTokens(tokensCacheR)} w=${humanTokens(tokensCacheW)}`);
    if (totalCost > 0) console.log(`  Cost:        $${totalCost.toFixed(6)}`);
  }
}

// ─── Rename ──────────────────────────────────────────────────────────────

async function rename(sessionId: string, name: string): Promise<void> {
  const client = new RunClient(grpcAddr());
  await client.renameSession(sessionId, name);
  console.log(`Renamed session ${sessionId} → "${name}"`);
}

// ─── Delete ───────────────────────────────────────────────────────────────

async function deleteSession(sessionId: string): Promise<void> {
  const client = new RunClient(grpcAddr());
  try {
    const { deleted } = await client.deleteSession(sessionId);
    if (deleted) {
      console.log(`Deleted session ${sessionId}`);
    } else {
      console.error(`Session not found: ${sessionId}`);
      process.exit(1);
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(msg.startsWith("failed to delete") ? msg : `Failed to delete: ${msg}`);
    process.exit(1);
  }
}

// ─── Entry ────────────────────────────────────────────────────────────────

export async function session(subcommand: string, args: string[]): Promise<void> {
  if (subcommand === "--help" || subcommand === "-h" || !subcommand) {
    help();
    return;
  }

  if (subcommand === "list") {
    await listSessions(args.includes("--json"));
    return;
  }

  const targetId = args[0];
  if (!targetId) {
    console.error(`Usage: future session ${subcommand} <session-id>${subcommand === "rename" ? " <name>" : ""}`);
    process.exit(1);
  }

  if (subcommand === "info") {
    await info(targetId);
    return;
  }

  if (subcommand === "rename") {
    const name = args.slice(1).join(" ");
    if (!name) {
      console.error("Usage: future session rename <session-id> <name>");
      process.exit(1);
    }
    await rename(targetId, name);
    return;
  }

  if (subcommand === "delete") {
    await deleteSession(targetId);
    return;
  }

  console.error(`Unknown command: ${subcommand}`);
  help();
  process.exit(1);
}
