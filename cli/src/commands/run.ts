/**
 * `future run` command — non-interactive agent execution.
 *
 * Usage:
 *   future run [options] [@files...] [message...]
 *
 * This replaces the TUI's print mode (-p/--print) with a dedicated CLI command
 * that supports fork, model/thinking selection, tool/skill scoping, and
 * permission levels.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { RunClient } from "../rpc/grpc-client.js";
import type { RunConfig } from "../rpc/grpc-client.js";
import type { ThinkingLevel, PermissionLevel } from "../rpc/types.js";

// ─── CLI Types ────────────────────────────────────────────────────────

interface RunArgs {
  grpcAddr: string;
  fork: string | null;
  session: string | null;
  continueLast: boolean;
  model: string | null;
  thinking: ThinkingLevel | null;
  tools: string[] | null;
  noTools: boolean;
  noBuiltinTools: boolean;
  systemPrompt: string | null;
  appendSystemPrompt: string[] | null;
  permission: PermissionLevel | null;
  noSession: boolean;
  mode: "text" | "json";
  cwd: string | null;
  verbose: boolean;
  fileArgs: string[];
  messages: string[];
}

// ─── Valid Values ─────────────────────────────────────────────────────

const VALID_THINKING_LEVELS: ThinkingLevel[] = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
];
const VALID_PERMISSION_LEVELS: PermissionLevel[] = ["all", "workspace", "none"];

// ─── Help Text ─────────────────────────────────────────────────────────

function printRunHelp(): void {
  console.log(`future run — send a prompt to the Future agent and print the response

Usage:
  future run [options] [@files...] [message...]

Options:
  --grpc-addr <addr>       gRPC server address (default: 127.0.0.1:50051)
  --session <id>           Connect to a specific session
  --continue, -c           Continue the most recent session
  --fork <entry-id>        Fork from a session entry
  --model <model>          Model ID (supports model:thinking format, e.g. "sonnet:high")
  --thinking <level>       Thinking level: off, minimal, low, medium, high, xhigh
  --tools, -t <tools>      Comma-separated tool names to enable
  --no-tools, -nt          Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools only (keep extensions)
  --system-prompt <text>   Set system prompt
  --append-system-prompt <text> Append to system prompt
  --permission <level>     Permission level: all, workspace, none
  --no-session             Ephemeral mode (don't save session)
  --mode <mode>            Output mode: text (default), json
  --cwd <dir>              Working directory
  --verbose                Show progress to stderr
  --help, -h               Show this help

Arguments:
  @files...    File paths to include in prompt (content wrapped in <file> tags)
  message...   The prompt text

Examples:
  future run "Explain this codebase"
  future run --model sonnet:high "Review the changes"
  future run --fork abc123 --thinking high "Continue from fork"
  future run --tools read,shell "Read the README"
  future run --permission workspace --no-tools "Summarize AGENTS.md"
  future run --mode json "What is 2+2?"
  future run @README.md "Summarize this file"
`);
}

// ─── Arg Parser ────────────────────────────────────────────────────────

function parseRunArgs(args: string[]): RunArgs | null {
  const defaultAddr =
    process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";

  const result: RunArgs = {
    grpcAddr: defaultAddr,
    fork: null,
    session: null,
    continueLast: false,
    model: null,
    thinking: null,
    tools: null,
    noTools: false,
    noBuiltinTools: false,
    systemPrompt: null,
    appendSystemPrompt: null,
    permission: null,
    noSession: false,
    mode: "text",
    cwd: null,
    verbose: false,
    fileArgs: [],
    messages: [],
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    switch (arg) {
      case "--grpc-addr":
        if (i + 1 < args.length) result.grpcAddr = args[++i];
        break;
      case "--fork":
        if (i + 1 < args.length) result.fork = args[++i];
        break;
      case "--session":
        if (i + 1 < args.length) result.session = args[++i];
        break;
      case "--continue":
      case "-c":
        result.continueLast = true;
        break;
      case "--model":
        if (i + 1 < args.length) {
          const modelArg = args[++i];
          const colonIndex = modelArg.lastIndexOf(":");
          if (colonIndex > 0) {
            const potentialThinking = modelArg.slice(colonIndex + 1);
            if (VALID_THINKING_LEVELS.includes(potentialThinking as ThinkingLevel)) {
              result.model = modelArg.slice(0, colonIndex);
              result.thinking = potentialThinking as ThinkingLevel;
            } else {
              result.model = modelArg;
            }
          } else {
            result.model = modelArg;
          }
        }
        break;
      case "--thinking":
        if (i + 1 < args.length) {
          const level = args[++i];
          if (!VALID_THINKING_LEVELS.includes(level as ThinkingLevel)) {
            console.error(
              `Invalid thinking level: ${level}. Valid: ${VALID_THINKING_LEVELS.join(", ")}`,
            );
            return null;
          }
          result.thinking = level as ThinkingLevel;
        }
        break;
      case "--tools":
      case "-t":
        if (i + 1 < args.length) {
          result.tools = args[++i].split(",").map((s) => s.trim());
        }
        break;
      case "--no-tools":
      case "-nt":
        result.noTools = true;
        break;
      case "--no-builtin-tools":
      case "-nbt":
        result.noBuiltinTools = true;
        break;
      case "--system-prompt":
        if (i + 1 < args.length) result.systemPrompt = args[++i];
        break;
      case "--append-system-prompt":
        result.appendSystemPrompt = result.appendSystemPrompt ?? [];
        if (i + 1 < args.length) result.appendSystemPrompt.push(args[++i]);
        break;
      case "--permission":
        if (i + 1 < args.length) {
          const level = args[++i];
          if (!VALID_PERMISSION_LEVELS.includes(level as PermissionLevel)) {
            console.error(
              `Invalid permission level: ${level}. Valid: ${VALID_PERMISSION_LEVELS.join(", ")}`,
            );
            return null;
          }
          result.permission = level as PermissionLevel;
        }
        break;
      case "--no-session":
        result.noSession = true;
        break;
      case "--mode":
        if (i + 1 < args.length) {
          const mode = args[++i];
          if (mode !== "text" && mode !== "json") {
            console.error(`Invalid mode: ${mode}. Valid: text, json`);
            return null;
          }
          result.mode = mode;
        }
        break;
      case "--cwd":
        if (i + 1 < args.length) result.cwd = args[++i];
        break;
      case "--verbose":
        result.verbose = true;
        break;
      case "--help":
      case "-h":
        printRunHelp();
        return null;
      default:
        if (arg.startsWith("@")) {
          result.fileArgs.push(arg.slice(1));
        } else if (arg.startsWith("-")) {
          console.error(`Unknown option: ${arg}`);
          return null;
        } else {
          result.messages.push(arg);
        }
        break;
    }
  }

  return result;
}

// ─── Prompt Builder ────────────────────────────────────────────────────

async function buildPrompt(
  fileArgs: string[],
  messages: string[],
): Promise<string | null> {
  if (fileArgs.length === 0 && messages.length === 0) {
    return null;
  }
  const parts: string[] = [];
  for (const filePath of fileArgs) {
    try {
      const absPath = path.resolve(filePath);
      const content = fs.readFileSync(absPath, "utf-8");
      parts.push(`<file name="${absPath}">\n${content}\n</file>`);
    } catch (e) {
      console.error(`Failed to read file: ${filePath}`);
      return null;
    }
  }
  parts.push(...messages);
  return parts.join("\n");
}

// ─── Main Command ──────────────────────────────────────────────────────

export async function run(args: string[]): Promise<void> {
  const parsed = parseRunArgs(args);

  // null means help was printed or parse error
  if (parsed === null) {
    return;
  }

  // Build prompt
  const prompt = await buildPrompt(parsed.fileArgs, parsed.messages);
  if (!prompt) {
    console.error(
      'No prompt provided. Usage: future run [options] [@files...] [message...]',
    );
    process.exit(1);
  }

  // Build RunConfig
  const runConfig: RunConfig = {
    grpcAddr: parsed.grpcAddr,
    fork: parsed.fork ?? undefined,
    session: parsed.session ?? undefined,
    continueLast: parsed.continueLast || undefined,
    model: parsed.model ?? undefined,
    thinking: parsed.thinking ?? undefined,
    tools: parsed.tools ?? undefined,
    noTools: parsed.noTools || undefined,
    noBuiltinTools: parsed.noBuiltinTools || undefined,
    systemPrompt: parsed.systemPrompt ?? undefined,
    appendSystemPrompt: parsed.appendSystemPrompt
      ? parsed.appendSystemPrompt.join("\n")
      : undefined,
    permission: parsed.permission ?? undefined,
    noSession: parsed.noSession || undefined,
    mode: parsed.mode,
    cwd: parsed.cwd ?? process.cwd(),
    verbose: parsed.verbose || undefined,
    message: prompt,
  };

  // Execute
  const client = new RunClient(parsed.grpcAddr);

  try {
    await client.run(runConfig);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    if (parsed.mode === "json") {
      console.log(JSON.stringify({ error: msg }));
    } else {
      console.error(`Error: ${msg}`);
    }
    process.exit(1);
  }
}
