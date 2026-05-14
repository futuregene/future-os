/**
 * xihu_tui TypeScript TUI entry point.
 *
 * Usage:
 *   node dist/index.js [options] [@files...] [messages...]
 *
 * Options:
 *   --grpc-addr <addr>     gRPC server address (default: localhost:50051)
 *   --session <id>         Connect to specific session
 *   --continue, -c         Continue most recent session
 *   --resume, -r           Resume a session (show picker)
 *   --fork <id>           Fork from a session
 *   --print, -p            Non-interactive mode: process prompt and exit
 *   --model <model>       Model to use
 *   --provider <provider>  Provider to use
 *   --list-models [search] List available models
 *   --thinking <level>     Thinking level: off, minimal, low, medium, high, xhigh
 *   --system-prompt <text> System prompt
 *   --tools <tools>       Comma-separated tool names to enable
 *   --no-tools            Disable all tools
 *   --no-session          Ephemeral mode (don't save session)
 *   --version, -v         Show version number
 *   --help, -h            Show this help
 *
 * Examples:
 *   # Interactive mode
 *   node dist/index.js
 *
 *   # With specific model
 *   node dist/index.js --model deepseek-v4-flash
 *
 *   # List models
 *   node dist/index.js --list-models
 *
 *   # Non-interactive with thinking level
 *   node dist/index.js -p --thinking high "Solve this"
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { App } from "./app.js";
import { GrpcClient } from "./rpc/grpc-client.js";
// ─── CLI Parsing ─────────────────────────────────────────────────
function parseArgs(args) {
    const result = {
        grpcAddr: process.env.XIHU_GRPC_ADDR ?? "localhost:50051",
        session: null,
        continue: false,
        resume: false,
        fork: null,
        print: false,
        fileArgs: [],
        messages: [],
        model: null,
        models: null,
        provider: null,
        apiKey: null,
        listModels: false,
        thinking: null,
        systemPrompt: null,
        appendSystemPrompt: null,
        tools: null,
        noTools: false,
        noBuiltinTools: false,
        noSession: false,
        mode: null,
        theme: null,
        noThemes: false,
        promptTemplate: null,
        noPromptTemplates: false,
        noContextFiles: false,
        offline: false,
        verbose: false,
        skill: null,
        noSkills: false,
        version: false,
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
            case "--model":
                if (i + 1 < args.length) {
                    const modelArg = args[++i];
                    // Support model:thinking format (e.g., sonnet:high, haiku:medium)
                    const colonIndex = modelArg.lastIndexOf(":");
                    if (colonIndex > 0) {
                        const potentialThinking = modelArg.slice(colonIndex + 1);
                        // Check if it looks like a thinking level
                        const thinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh"];
                        if (thinkingLevels.includes(potentialThinking)) {
                            result.model = modelArg.slice(0, colonIndex);
                            result.thinking = potentialThinking;
                        }
                        else {
                            result.model = modelArg;
                        }
                    }
                    else {
                        result.model = modelArg;
                    }
                }
                break;
            case "--models":
                if (i + 1 < args.length) {
                    result.models = args[++i].split(",").map((s) => s.trim());
                }
                break;
            case "--provider":
                if (i + 1 < args.length) {
                    result.provider = args[++i];
                }
                break;
            case "--api-key":
                if (i + 1 < args.length) {
                    result.apiKey = args[++i];
                }
                break;
            case "--append-system-prompt":
                result.appendSystemPrompt = result.appendSystemPrompt ?? [];
                if (i + 1 < args.length) {
                    result.appendSystemPrompt.push(args[++i]);
                }
                break;
            case "--list-models":
                result.listModels = true;
                if (i + 1 < args.length && !args[i + 1].startsWith("-") && !args[i + 1].startsWith("@")) {
                    result.listModels = args[++i];
                }
                break;
            case "--thinking":
                if (i + 1 < args.length) {
                    result.thinking = args[++i];
                }
                break;
            case "--system-prompt":
                if (i + 1 < args.length) {
                    result.systemPrompt = args[++i];
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
            case "--no-session":
                result.noSession = true;
                break;
            case "--mode":
                if (i + 1 < args.length) {
                    result.mode = args[++i];
                }
                break;
            case "--theme":
                if (i + 1 < args.length) {
                    result.theme = args[++i];
                }
                break;
            case "--no-themes":
                result.noThemes = true;
                break;
            case "--prompt-template":
                result.promptTemplate = result.promptTemplate ?? [];
                if (i + 1 < args.length) {
                    result.promptTemplate.push(args[++i]);
                }
                break;
            case "--no-prompt-templates":
            case "-np":
                result.noPromptTemplates = true;
                break;
            case "--no-context-files":
            case "-nc":
                result.noContextFiles = true;
                break;
            case "--offline":
                result.offline = true;
                break;
            case "--verbose":
                result.verbose = true;
                break;
            case "--skill":
                result.skill = result.skill ?? [];
                if (i + 1 < args.length) {
                    result.skill.push(args[++i]);
                }
                break;
            case "--no-skills":
            case "-ns":
                result.noSkills = true;
                break;
            case "--version":
            case "-v":
                result.version = true;
                break;
            case "--help":
            case "-h":
                printHelp();
                process.exit(0);
                break;
            default:
                if (arg.startsWith("@")) {
                    result.fileArgs.push(arg.slice(1));
                }
                else if (arg.startsWith("-")) {
                    console.error(`Unknown option: ${arg}`);
                    process.exit(1);
                }
                else {
                    result.messages.push(arg);
                }
                break;
        }
    }
    return result;
}
function printHelp() {
    console.log(`xihu_tui TUI

Usage: node dist/index.js [options] [@files...] [messages...]

Options:
  --grpc-addr <addr>    gRPC server address (default: localhost:50051)
  --session <id>        Connect to specific session
  --continue, -c        Continue most recent session
  --resume, -r          Resume a session (show picker)
  --fork <id>           Fork from a session
  --print, -p           Non-interactive mode: process prompt and exit
  --model <model>       Model to use (supports model:thinking format)
  --models <patterns>   Model patterns for Ctrl+P cycling (comma-separated, supports globs)
  --provider <provider>  Provider to use
  --api-key <key>       API key (overrides env vars)
  --list-models [search] List available models (with optional search)
  --thinking <level>    Thinking level: off, minimal, low, medium, high, xhigh
  --system-prompt <text> Set system prompt
  --append-system-prompt <text> Append to system prompt
  --tools, -t <tools>  Comma-separated tool names to enable
  --no-tools, -nt       Disable all tools
  --no-builtin-tools, -nbt Disable built-in tools (keep extensions)
  --no-session          Ephemeral mode (don't save session)
  --mode <mode>        Output mode: text, json (default: text)
  --theme <path>       Load a theme file
  --no-themes          Disable themes
  --prompt-template <path> Load a prompt template file
  --no-prompt-templates, -np Disable prompt templates
  --no-context-files, -nc  Disable AGENTS.md and CLAUDE.md discovery
  --offline             Disable startup network operations
  --verbose             Show detailed startup information
  --skill <path>        Load a skill file or directory
  --no-skills, -ns      Disable skills discovery
  --version, -v         Show version number
  --help, -h            Show this help

Examples:
  # Interactive mode
  node dist/index.js

  # With specific model
  node dist/index.js --model deepseek-v4-flash

  # Model with thinking level (model:thinking format)
  node dist/index.js --model sonnet:high

  # List models
  node dist/index.js --list-models

  # List models with search
  node dist/index.js --list-models deepseek

  # Non-interactive with thinking level
  node dist/index.js -p --thinking high "Solve this problem"

  # Enable only read and bash tools
  node dist/index.js --tools read,bash -p "Review this code"

  # JSON output mode
  node dist/index.js --mode json -p "What is 2+2?"
`);
}
// ─── Build Initial Prompt ─────────────────────────────────────────────
async function buildInitialPrompt(fileArgs, messages) {
    if (fileArgs.length === 0 && messages.length === 0) {
        return undefined;
    }
    const promptParts = [];
    for (const filePath of fileArgs) {
        try {
            const absPath = path.resolve(filePath);
            const content = fs.readFileSync(absPath, "utf-8");
            promptParts.push(`<file name="${absPath}">\n${content}\n</file>`);
        }
        catch (e) {
            console.error(`Failed to read file: ${filePath}`);
            return undefined;
        }
    }
    promptParts.push(...messages);
    return promptParts.join("\n");
}
// ─── Apply CLI Options to Server ─────────────────────────────────
async function applyCliOptions(client, sessionId, args) {
    // Set model first (provider/model)
    if (args.model) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg1", type: "set_model", modelId: args.model, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Set thinking level
    if (args.thinking) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg2", type: "set_thinking_level", level: args.thinking, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Set system prompt
    if (args.systemPrompt) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg3", type: "set_system_prompt", systemPrompt: args.systemPrompt, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Set tools
    if (args.tools && args.tools.length > 0) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg4", type: "set_tools", tools: args.tools, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Disable tools
    if (args.noTools) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg5", type: "disable_tools", sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Set ephemeral mode
    if (args.noSession) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg6", type: "set_ephemeral", ephemeral: true, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Disable built-in tools (keep extensions)
    if (args.noBuiltinTools) {
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg7", type: "disable_builtin_tools", sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
    // Append system prompt
    if (args.appendSystemPrompt && args.appendSystemPrompt.length > 0) {
        const prompt = args.appendSystemPrompt.join("\n");
        await new Promise((resolve, reject) => {
            client.ExecuteCommand({ id: "cfg8", type: "append_system_prompt", systemPrompt: prompt, sessionId }, (err, response) => {
                if (err || !response.success)
                    reject(new Error(response?.error || err?.message));
                else
                    resolve();
            });
        });
    }
}
// ─── Print Mode (Non-Interactive) ─────────────────────────────────
async function runPrintMode(grpcAddr, fileArgs, messages, args) {
    const prompt = await buildInitialPrompt(fileArgs, messages);
    if (!prompt) {
        console.error("No prompt provided");
        process.exit(1);
    }
    // Connect to gRPC server
    const client = new GrpcClient(grpcAddr);
    // Get initial state to get session ID
    const state = await new Promise((resolve, reject) => {
        const request = { id: String(Date.now()), type: "get_state", sessionId: "" };
        client.client.ExecuteCommand(request, (err, response) => {
            if (err || !response.success) {
                reject(new Error(response?.error || err?.message || "get_state failed"));
            }
            else {
                resolve(JSON.parse(response.data));
            }
        });
    });
    const sessionId = state.sessionId;
    const isJsonMode = args.mode === "json";
    // Apply CLI options
    await applyCliOptions(client.client, sessionId, args);
    // JSON mode response accumulator
    const jsonMessages = [];
    // Subscribe to events BEFORE sending prompt
    const stream = client.client.StreamEvents({ session_id: sessionId });
    let text = "";
    let done = false;
    // Wait for agent_end event
    const eventPromise = new Promise((resolve, reject) => {
        stream.on("data", (response) => {
            if (isJsonMode) {
                // JSON mode: accumulate all events
                jsonMessages.push(JSON.parse(response.data));
                if (response.type === "agent_end") {
                    done = true;
                    stream.cancel();
                    resolve();
                }
            }
            else {
                // Text mode: accumulate, output only at end
                if (response.type === "text_chunk") {
                    const data = JSON.parse(response.data);
                    text += data.text ?? "";
                }
                else if (response.type === "agent_end") {
                    done = true;
                    stream.cancel();
                    resolve();
                }
            }
        });
        stream.on("error", (err) => {
            if (!done) {
                reject(err);
            }
        });
    });
    // Send prompt
    await new Promise((resolve, reject) => {
        const request = {
            id: String(Date.now()),
            type: "prompt",
            sessionId,
            message: prompt,
        };
        client.client.ExecuteCommand(request, (err, response) => {
            if (err || !response.success) {
                stream.cancel();
                reject(new Error(response?.error || err?.message || "prompt failed"));
            }
            else {
                resolve();
            }
        });
    });
    // Wait for response to complete
    await eventPromise;
    // Output result
    if (isJsonMode) {
        // JSON mode: output all events
        const result = {
            sessionId,
            messages: jsonMessages,
        };
        console.log(JSON.stringify(result, null, 2));
    }
    else {
        // Text mode: output final text
        if (text) {
            console.log(text);
        }
    }
}
// ─── List Models ─────────────────────────────────────────────────
async function listModels(grpcAddr, search) {
    const client = new GrpcClient(grpcAddr);
    const result = await new Promise((resolve, reject) => {
        client.client.ExecuteCommand({ id: "1", type: "get_available_models" }, (err, response) => {
            if (err || !response.success) {
                reject(new Error(response?.error || err?.message));
            }
            else {
                resolve(JSON.parse(response.data));
            }
        });
    });
    let models = result.models || [];
    if (search) {
        const searchLower = search.toLowerCase();
        models = models.filter((m) => m.toLowerCase().includes(searchLower));
    }
    console.log(`Found ${models.length} model(s):`);
    for (const model of models.slice(0, 50)) {
        console.log(`  ${model}`);
    }
    if (models.length > 50) {
        console.log(`  ... and ${models.length - 50} more`);
    }
}
// ─── Main ────────────────────────────────────────────────────────
const args = parseArgs(process.argv.slice(2));
// Handle --version
if (args.version) {
    console.log("xihu_tui TUI v0.3.0");
    process.exit(0);
}
// Handle --list-models
if (args.listModels) {
    console.error(`Connecting to gRPC server at ${args.grpcAddr}`);
    listModels(args.grpcAddr, typeof args.listModels === "string" ? args.listModels : undefined)
        .then(() => process.exit(0))
        .catch((err) => {
        console.error("Error:", err.message);
        process.exit(1);
    });
}
// Print mode: non-interactive
if (args.print) {
    if (args.messages.length === 0 && args.fileArgs.length === 0) {
        if (args.mode !== "json") {
            console.error("No prompt provided. Usage: xihu_tui -p \"message\"");
        }
        process.exit(1);
    }
    runPrintMode(args.grpcAddr, args.fileArgs, args.messages, args)
        .then(() => {
        process.exit(0);
    })
        .catch((err) => {
        if (args.mode !== "json") {
            console.error("Error:", err.message);
        }
        process.exit(1);
    });
}
else {
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
    // Apply CLI options after app starts
    app.start().then(() => {
        // Options are applied via gRPC after connection
        // For now, TUI reads options from state - server handles them
    }).catch((err) => {
        console.error("Fatal error:", err);
        process.exit(1);
    });
}
