#!/usr/bin/env node

import { credential, login, logout, status } from "./commands/auth.js";
import { tools, isToolsCommand } from "./commands/tools.js";
import { skills, isSkillsCommand } from "./commands/skills.js";
import { account, isAccountCommand } from "./commands/account.js";
import { run as runCommand } from "./commands/run.js";
import { docker } from "./commands/docker.js";
import { printHelp } from "./help.js";
import { VERSION } from "./version.generated.js";

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const [group, command, ...rest] = args;

  if (group === "--version" || group === "-v" || group === "version") {
    console.log(`future v${VERSION}`);
    return;
  }

  if (group === "auth" && command === "login") {
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
    await status();
    return;
  }

  if (group === "auth" && command === "credential") {
    const jsonFlag = rest.includes("--json");
    await credential({ json: jsonFlag });
    return;
  }

  if (group === "auth" && command === "logout") {
    await logout();
    return;
  }

  if (group === "tools" && isToolsCommand(command)) {
    await tools(command, args.slice(2));
    return;
  }

  if (group === "skills" && isSkillsCommand(command)) {
    await skills(command, args.slice(2));
    return;
  }

  if (group === "account" && isAccountCommand(command)) {
    await account(command, rest);
    return;
  }

  if (group === "run") {
    await runCommand(args.slice(1));
    return;
  }

  if (group === "doctor") {
    const restArgs = args.slice(1);
    const fix = restArgs.includes("--fix");
    await docker(fix);
    return;
  }

  printHelp();
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
