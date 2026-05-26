#!/usr/bin/env node
import { agent, isAgentCommand } from "./commands/agent.js";
import { login, logout, status } from "./commands/auth.js";
import { tui } from "./commands/tui.js";
import { printHelp } from "./help.js";
async function main() {
    const args = process.argv.slice(2);
    const [group, command] = args;
    if (group === "auth" && command === "login") {
        await login();
        return;
    }
    if (group === "auth" && command === "status") {
        await status();
        return;
    }
    if (group === "auth" && command === "logout") {
        await logout();
        return;
    }
    if (group === "agent" && isAgentCommand(command)) {
        await agent(command);
        return;
    }
    if (group === "tui") {
        await tui(args.slice(1));
        return;
    }
    printHelp();
}
main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
});
