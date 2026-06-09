#!/usr/bin/env node
import { agent, isAgentCommand } from "./commands/agent.js";
import { channel, isChannelCommand } from "./commands/channel.js";
import { login, logout, status } from "./commands/auth.js";
import { tui } from "./commands/tui.js";
import { tools, isToolsCommand } from "./commands/tools.js";
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
    if (group === "channel" && isChannelCommand(command)) {
        await channel(command);
        return;
    }
    if (group === "tui") {
        await tui(args.slice(1));
        return;
    }
    if (group === "tools" && isToolsCommand(command)) {
        await tools(command, args.slice(2));
        return;
    }
    printHelp();
}
main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
});
