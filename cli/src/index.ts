#!/usr/bin/env node

import { credential, login, logout, status } from "./commands/auth.js";
import { tools, isToolsCommand } from "./commands/tools.js";
import { skills, isSkillsCommand } from "./commands/skills.js";
import { account, isAccountCommand } from "./commands/account.js";
import { run as runCommand } from "./commands/run.js";
import { models } from "./commands/models.js";
import { session } from "./commands/session.js";
import { agentStatus } from "./commands/agent.js";
import { doctor } from "./commands/doctor.js";
import { printHelp } from "./help.js";
import { VERSION } from "./version.generated.js";

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const [group, command, ...rest] = args;

  if (group === "--version" || group === "-v" || group === "version") {
    console.log(`future v${VERSION}`);
    return;
  }

  if (group === "auth" && (!command || command === "--help" || command === "-h")) {
    console.log(`future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the API key + endpoint for shell scripts. Output is always JSON
              on success; use --json for consistent JSON error output when not logged in.
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)`);
    return;
  }

  if (group === "auth" && command === "login") {
    if (rest.includes("--help") || rest.includes("-h")) {
      console.log(`future auth login — device-code OAuth flow

Usage:
  future auth login [--url <url>]

  --url <url>   Override the platform URL (default from DNS TXT record or built-in)
  --help, -h    Show this help

Opens a browser for you to sign in and authorize this CLI device.
Saves the resulting API key to ~/.future/agent/auth.json.`);
      return;
    }
    const urlIdx = rest.indexOf("--url");
    let urlOverride: string | undefined;
    if (urlIdx !== -1 && urlIdx + 1 < rest.length) {
      urlOverride = rest[urlIdx + 1];
    } else {
      const urlEq = rest.find(a => a.startsWith("--url="));
      urlOverride = urlEq?.slice("--url=".length);
    }
    await login(urlOverride);
    return;
  }

  if (group === "auth" && command === "status") {
    if (rest.includes("--help") || rest.includes("-h")) {
      console.log("future auth status — check current login state\n\nShows the platform URL and indicates whether an API key is stored.\nDoes not validate the key against the server.");
      return;
    }
    await status();
    return;
  }

  if (group === "auth" && command === "credential") {
    if (rest.includes("--help") || rest.includes("-h")) {
      console.log(`future auth credential — output API key for scripting

Usage:
  future auth credential [--json]

Output (always JSON on success):
  {"api_key":"...","endpoint":"..."}

  --json    When not logged in, emit JSON error instead of plain text.
            On success the output is always JSON regardless of this flag.

Useful for piping into other tools or CI/CD scripts.`);
      return;
    }
    const jsonFlag = rest.includes("--json");
    await credential({ json: jsonFlag });
    return;
  }

  if (group === "auth" && command === "logout") {
    if (rest.includes("--help") || rest.includes("-h")) {
      console.log("future auth logout — remove stored API key\n\nDeletes the Future provider key from ~/.future/agent/auth.json.\nOther provider keys in the file are left untouched.");
      return;
    }
    await logout();
    return;
  }

  // Unknown subcommand under "auth" — show group help
  if (group === "auth") {
    console.error(`Unknown command: ${command}\n`);
    console.log(`future auth — authenticate with the Future platform

Usage:
  future auth <command>

Commands:
  login       Device-code OAuth flow; saves API key to ~/.future/agent/auth.json
  status      Show whether logged in, and the platform URL in use
  credential  Output the raw API key + endpoint for shell scripts (--json not needed;
              output is always JSON: {"api_key":"...","endpoint":"..."})
  logout      Remove the stored API key from auth.json

API key file: ~/.future/agent/auth.json
Environment override: FUTURE_API_KEY (takes precedence over auth.json)`);
    return;
  }

  if (group === "tools" && (!command || command === "--help" || command === "-h")) {
    console.log(`future tools — list, describe, and call platform & browser tools

Usage:
  future tools list [--json]
  future tools describe <name>
  future tools call <name> --key1 val1 --key2 val2 [...]

Commands:
  list               Show available tools. --json for machine output.
  describe <name>    Show a tool's arguments and usage example.
  call <name>        Invoke a tool. Args as --key value. Use describe first to see
                     what arguments each tool accepts.

Requires authentication: future auth login, or set FUTURE_API_KEY.`);
    return;
  }

  if (group === "tools" && isToolsCommand(command)) {
    await tools(command, args.slice(2));
    return;
  }

  // Unknown subcommand under "tools" — show group help
  if (group === "tools") {
    console.error(`Unknown command: ${command}\n`);
    console.log(`future tools — list, describe, and call platform & browser tools

Usage:
  future tools list [--json]
  future tools describe <name>
  future tools call <name> --key1 val1 --key2 val2 [...]

Commands:
  list               Show available tools. --json for machine output.
  describe <name>    Show a tool's arguments and usage example.
  call <name>        Invoke a tool. Args as --key value. Use describe first to see
                     what arguments each tool accepts.

Requires authentication: future auth login, or set FUTURE_API_KEY.`);
    return;
  }

  if (group === "skills" && (!command || command === "--help" || command === "-h")) {
    console.log(`future skills — install & manage agent skills

Skills are markdown instruction files the agent loads to handle specific tasks.
They live under ~/.future/agent/skills/<name>/SKILL.md.

Usage:
  future skills <command> [args]

Commands:
  list                    Show all skills available in the catalog (name, latest version,
                          installed version, description).
  install <name>          Install a specific skill by name. Use --version <ver> for a
                          specific version; omit for latest.
  install                 With no name argument, same as install-builtin.
  install-builtin         Install all built-in platform skills (names prefixed "future-").
  uninstall <name>        Remove an installed skill.
  update                  Upgrade all installed skills to their latest versions.

Skills directory: ~/.future/agent/skills/
Catalog source: fetched from the Future platform API.`);
    return;
  }

  if (group === "skills" && isSkillsCommand(command)) {
    await skills(command, args.slice(2));
    return;
  }

  // Unknown subcommand under "skills" — show group help
  if (group === "skills") {
    console.error(`Unknown command: ${command}\n`);
    console.log(`future skills — install & manage agent skills

Skills are markdown instruction files the agent loads to handle specific tasks.
They live under ~/.future/agent/skills/<name>/SKILL.md.

Usage:
  future skills <command> [args]

Commands:
  list                    Show all skills available in the catalog (name, latest version,
                          installed version, description).
  install <name>          Install a specific skill by name. Use --version <ver> for a
                          specific version; omit for latest.
  install                 With no name argument, same as install-builtin.
  install-builtin         Install all built-in platform skills (names prefixed "future-").
  uninstall <name>        Remove an installed skill.
  update                  Upgrade all installed skills to their latest versions.

Skills directory: ~/.future/agent/skills/
Catalog source: fetched from the Future platform API.`);
    return;
  }

  if (group === "account" && (!command || command === "--help" || command === "-h")) {
    console.log(`future account — view platform account information

Usage:
  future account <command>

Commands:
  profile     Show account profile (email, user ID, verification status, creation date)
  balance     Show account credit balance. Use --json for machine-readable output.

Requires authentication: future auth login first.`);
    return;
  }

  if (group === "account" && isAccountCommand(command)) {
    await account(command, rest);
    return;
  }

  // Unknown subcommand under "account" — show group help
  if (group === "account") {
    console.error(`Unknown command: ${command}\n`);
    console.log(`future account — view platform account information

Usage:
  future account <command>

Commands:
  profile     Show account profile (email, user ID, verification status, creation date)
  balance     Show account credit balance. Use --json for machine-readable output.

Requires authentication: future auth login first.`);
    return;
  }

  if (group === "run") {
    await runCommand(args.slice(1));
    return;
  }

  if (group === "models") {
    if (command === "--help" || command === "-h" || rest.includes("--help") || rest.includes("-h")) {
      console.log(`future models — list available models from the running agent

Usage:
  future models [--json]

  --json    Output as JSON array with id, label, provider, contextWindow,
            supportsImages, thinkingLevel, and isDefault fields.
  --help    Show this help.

Requires a running agent (connects to 127.0.0.1:50051 by default).
Override with FUTURE_AGENT_GRPC_ADDR environment variable.`);
      return;
    }
    await models(command === "--json" ? [command, ...rest] : rest);
    return;
  }

  if (group === "agent") {
    if (command === "status" || !command) {
      if (rest.includes("--help") || rest.includes("-h") || command === "--help" || command === "-h") {
        console.log(`future agent status — show running agent state

Usage:
  future agent status [--json]

  --json    Output full state as JSON.
  --help    Show this help.`);
        return;
      }
      await agentStatus(command === "--json" || rest.includes("--json"));
      return;
    }
    console.error(`Unknown command: ${command}\n`);
    console.error(`Usage: future agent status [--json]`);
    process.exit(1);
    return;
  }

  if (group === "session") {
    await session(command, rest);
    return;
  }

  if (group === "doctor") {
    await doctor();
    return;
  }

  printHelp();
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
