/**
 * xihu TypeScript TUI entry point.
 *
 * Usage:
 *   node dist/index.js [options] [@files...] [messages...]
 *
 * Options:
 *   --grpc-addr <addr>     gRPC server address (default: localhost:50051)
 *   --session <id>         Connect to specific session
 *   --continue, -c         Continue most recent session
 *   --resume, -r           Resume a session (show picker)
 *   --fork <id>            Fork from a session
 *   --print, -p            Non-interactive mode: process prompt and exit
 *   --help, -h             Show this help
 *
 * Examples:
 *   # Interactive mode
 *   node dist/index.js
 *
 *   # Interactive mode with initial prompt
 *   node dist/index.js "List all .ts files in src/"
 *
 *   # Include files in initial message
 *   node dist/index.js @prompt.md "What does this do?"
 *
 *   # Non-interactive mode (process and exit)
 *   node dist/index.js -p "List all .ts files in src/"
 *
 *   # With session
 *   node dist/index.js --session 20260514-140838-1a064f
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { App } from "./app.js";
import { GrpcClient } from "./rpc/grpc-client.js";

// ─── CLI Types ─────────────────────────────────────────────────────

interface CliArgs {
  grpcAddr: string;
  session: string | null;
  continue: boolean;
  resume: boolean;
  fork: string | null;
  print: boolean;
  fileArgs: string[];
  messages: string[];
}

// ─── CLI Parsing ─────────────────────────────────────────────────

function parseArgs(args: string[]): CliArgs {
  const result: CliArgs = {
    grpcAddr: process.env.XIHU_GRPC_ADDR ?? "localhost:50051",
    session: null,
    continue: false,
    resume: false,
    fork: null,
    print: false,
    fileArgs: [],
    messages: [],
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    switch (arg) {
      case "--grpc-addr":
        if (i + 1 < args.length) {
          result.grpcAddr = args[++i];
        }
        break;
      case "--session":
        if (i + 1 < args.length) {
          result.session = args[++i];
        }
        break;
      case "--continue":
      case "-c":
        result.continue = true;
        break;
      case "--resume":
      case "-r":
        result.resume = true;
        break;
      case "--fork":
        if (i + 1 < args.length) {
          result.fork = args[++i];
        }
        break;
      case "--print":
      case "-p":
        result.print = true;
        // Check if next arg is a message (not a flag or file arg)
        if (i + 1 < args.length && !args[i + 1].startsWith("@") && !args[i + 1].startsWith("-")) {
          result.messages.push(args[++i]);
        }
        break;
      case "--help":
      case "-h":
        printHelp();
        process.exit(0);
        break;
      default:
        if (arg.startsWith("@")) {
          // File argument: @filename
          result.fileArgs.push(arg.slice(1));
        } else if (arg.startsWith("-")) {
          console.error(`Unknown option: ${arg}`);
          process.exit(1);
        } else {
          result.messages.push(arg);
        }
        break;
    }
  }

  return result;
}

function printHelp(): void {
  console.log(`xihu TUI

Usage: node dist/index.js [options] [@files...] [messages...]

Options:
  --grpc-addr <addr>   gRPC server address (default: localhost:50051)
  --session <id>       Connect to specific session
  --continue, -c       Continue most recent session
  --resume, -r         Resume a session (show picker)
  --fork <id>           Fork from a session
  --print, -p           Non-interactive mode: process prompt and exit
  --help, -h            Show this help

Examples:
  # Interactive mode
  node dist/index.js

  # Interactive mode with initial prompt
  node dist/index.js "List all .ts files in src/"

  # Include files in initial message
  node dist/index.js @prompt.md "What does this do?"

  # Non-interactive mode (process and exit)
  node dist/index.js -p "List all .ts files in src/"
`);
}

// ─── Print Mode (Non-Interactive) ─────────────────────────────────

async function runPrintMode(
  grpcAddr: string,
  fileArgs: string[],
  messages: string[],
): Promise<void> {
  // Build the prompt: file contents + messages
  const promptParts: string[] = [];

  // Include files
  for (const filePath of fileArgs) {
    try {
      const absPath = path.resolve(filePath);
      const content = fs.readFileSync(absPath, "utf-8");
      promptParts.push(`<file name="${absPath}">\n${content}\n</file>`);
    } catch (e) {
      console.error(`Failed to read file: ${filePath}`);
      process.exit(1);
    }
  }

  // Add messages
  promptParts.push(...messages);

  const prompt = promptParts.join("\n");

  // Connect to gRPC server
  const client = new GrpcClient(grpcAddr);

  // Get initial state to get session ID
  const state = await new Promise<any>((resolve, reject) => {
    const request = { id: String(Date.now()), type: "get_state", sessionId: "" };
    (client as any).client.ExecuteCommand(request, (err: Error | null, response: any) => {
      if (err || !response.success) {
        reject(new Error(response?.error || err?.message || "get_state failed"));
      } else {
        resolve(JSON.parse(response.data));
      }
    });
  });

  const sessionId = state.sessionId;
  console.error(`Connected to session: ${sessionId}`);

  // Subscribe to events BEFORE sending prompt
  const stream = (client as any).client.StreamEvents({ session_id: sessionId });

  let text = "";
  let done = false;

  // Wait for agent_end event
  const eventPromise = new Promise<void>((resolve, reject) => {
    stream.on("data", (response: any) => {
      if (response.type === "text_chunk") {
        const data = JSON.parse(response.data);
        text += data.text ?? "";
        process.stdout.write(data.text ?? "");
      } else if (response.type === "agent_end") {
        done = true;
        stream.cancel();
        resolve();
      }
    });

    stream.on("error", (err: Error) => {
      if (!done) {
        reject(err);
      }
    });
  });

  // Send prompt
  await new Promise<void>((resolve, reject) => {
    const request = {
      id: String(Date.now()),
      type: "prompt",
      sessionId,
      message: prompt,
    };
    (client as any).client.ExecuteCommand(request, (err: Error | null, response: any) => {
      if (err || !response.success) {
        stream.cancel();
        reject(new Error(response?.error || err?.message || "prompt failed"));
      } else {
        resolve();
      }
    });
  });

  // Wait for response to complete
  await eventPromise;
}

// ─── Main ────────────────────────────────────────────────────────

const args = parseArgs(process.argv.slice(2));

// Print mode: non-interactive
if (args.print) {
  if (args.messages.length === 0 && args.fileArgs.length === 0) {
    console.error("No prompt provided. Usage: xihu -p \"message\"");
    process.exit(1);
  }
  console.error(`Connecting to gRPC server at ${args.grpcAddr}`);
  runPrintMode(args.grpcAddr, args.fileArgs, args.messages)
    .then(() => {
      process.exit(0);
    })
    .catch((err) => {
      console.error("Error:", err.message);
      process.exit(1);
    });
} else {
  // Interactive mode (TUI)
  console.error(`Connecting to gRPC server at ${args.grpcAddr}`);

  const app = new App(args.grpcAddr, {
    session: args.session,
    continue: args.continue,
    resume: args.resume,
    fork: args.fork,
  });

  process.on("SIGINT", async () => {
    await app.stop();
    process.exit(0);
  });

  process.on("SIGTERM", async () => {
    await app.stop();
    process.exit(0);
  });

  app.start().catch((err) => {
    console.error("Fatal error:", err);
    process.exit(1);
  });
}
