import { chmod, mkdir, writeFile } from "node:fs/promises";
import { homedir, platform as osPlatform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { DEFAULT_AGENT_GRPC_ADDR, DEFAULT_LAUNCHD_LABEL, DEFAULT_SYSTEMD_UNIT, DEFAULT_WINDOWS_SERVICE, } from "../constants.js";
import { assertExecutableFile, canAccess, fsConstants } from "../utils/files.js";
import { formatProcessOutput, runProcess } from "../utils/process.js";
import { escapeXml } from "../utils/string.js";
export async function agent(command) {
    const platform = osPlatform();
    if (platform === "darwin") {
        await runDarwinAgentCommand(command);
        return;
    }
    if (platform === "linux") {
        await runLinuxAgentCommand(command);
        return;
    }
    if (platform === "win32") {
        await runWindowsAgentCommand(command);
        return;
    }
    throw new Error(`Unsupported platform for agent service management: ${platform}`);
}
export function isAgentCommand(command) {
    return command === "start" || command === "stop" || command === "restart" || command === "status";
}
async function runDarwinAgentCommand(command) {
    const label = resolveServiceName(DEFAULT_LAUNCHD_LABEL, "FUTURE_AGENT_LAUNCHD_LABEL");
    const target = `gui/${getUserId()}/${label}`;
    if (command === "start") {
        await ensureDarwinLaunchAgent(label, target);
        await runServiceCommand("launchctl", ["kickstart", "-k", target], {
            successMessage: `Started Future agent service (${label}).`,
        });
        return;
    }
    if (command === "stop") {
        await runServiceCommand("launchctl", ["bootout", target], {
            failureHint: `Future agent launchd service is not loaded (${label}).`,
            successMessage: `Stopped Future agent service (${label}).`,
        });
        return;
    }
    if (command === "restart") {
        if (await isDarwinLaunchAgentLoaded(target)) {
            await runServiceCommand("launchctl", ["bootout", target]);
        }
        await ensureDarwinLaunchAgent(label, target);
        await runServiceCommand("launchctl", ["kickstart", "-k", target], {
            successMessage: `Restarted Future agent service (${label}).`,
        });
        return;
    }
    await runServiceCommand("launchctl", ["print", target], {
        failureHint: `Future agent launchd service is not loaded (${label}).`,
        printOutput: true,
    });
}
async function ensureDarwinLaunchAgent(label, target) {
    if (await isDarwinLaunchAgentLoaded(target)) {
        return;
    }
    const plistPath = await writeDarwinLaunchAgentPlist(label);
    const result = await runProcess("launchctl", ["bootstrap", `gui/${getUserId()}`, plistPath]);
    if (result.code !== 0) {
        const output = formatProcessOutput(result);
        throw new Error(`Failed to bootstrap Future agent launchd service (${label}).${output ? `\n${output}` : ""}`);
    }
}
async function isDarwinLaunchAgentLoaded(target) {
    const result = await runProcess("launchctl", ["print", target]);
    return result.code === 0;
}
async function writeDarwinLaunchAgentPlist(label) {
    const agentBinary = await resolveAgentBinary();
    const grpcAddr = process.env.FUTURE_AGENT_GRPC_ADDR ?? DEFAULT_AGENT_GRPC_ADDR;
    const launchAgentsDir = join(homedir(), "Library", "LaunchAgents");
    const logDir = join(homedir(), ".future", "agent", "logs");
    const plistPath = join(launchAgentsDir, `${label}.plist`);
    await mkdir(launchAgentsDir, { recursive: true });
    await mkdir(logDir, { recursive: true });
    await writeFile(plistPath, launchAgentPlist({
        label,
        agentBinary,
        grpcAddr,
        stdoutPath: join(logDir, "agent.out.log"),
        stderrPath: join(logDir, "agent.err.log"),
    }), { mode: 0o644 });
    await chmod(plistPath, 0o644);
    return plistPath;
}
async function runLinuxAgentCommand(command) {
    const unit = resolveServiceName(DEFAULT_SYSTEMD_UNIT, "FUTURE_AGENT_SYSTEMD_UNIT");
    if (command === "status") {
        await runServiceCommand("systemctl", ["--user", "status", "--no-pager", unit], {
            failureHint: `Future agent systemd user service is not active or not installed (${unit}).`,
            printOutput: true,
        });
        return;
    }
    await runServiceCommand("systemctl", ["--user", command, unit], {
        successMessage: `${agentCommandPastTense(command)} Future agent service (${unit}).`,
    });
}
async function runWindowsAgentCommand(command) {
    const service = resolveServiceName(DEFAULT_WINDOWS_SERVICE, "FUTURE_AGENT_WINDOWS_SERVICE");
    if (command === "status") {
        await runServiceCommand("sc.exe", ["query", service], {
            failureHint: `Future agent Windows service is not installed (${service}).`,
            printOutput: true,
        });
        return;
    }
    if (command === "restart") {
        await runWindowsAgentCommand("stop");
        await runWindowsAgentCommand("start");
        return;
    }
    const action = command === "start" ? "start" : "stop";
    await runServiceCommand("sc.exe", [action, service], {
        successMessage: `${agentCommandPastTense(command)} Future agent service (${service}).`,
    });
}
async function runServiceCommand(command, args, options = {}) {
    const result = await runProcess(command, args);
    const output = formatProcessOutput(result);
    if (result.code !== 0) {
        if (options.failureHint) {
            console.error(options.failureHint);
        }
        if (output) {
            console.error(output);
        }
        process.exitCode = result.code;
        return result;
    }
    if (options.printOutput && output) {
        console.log(output);
    }
    else if (options.successMessage) {
        console.log(options.successMessage);
    }
    return result;
}
async function resolveAgentBinary() {
    const override = process.env.FUTURE_AGENT_BIN;
    if (override) {
        await assertExecutableFile(override, "FUTURE_AGENT_BIN");
        return override;
    }
    const currentFile = fileURLToPath(import.meta.url);
    const cliRoot = resolve(dirname(currentFile), "..", "..");
    const repoRoot = resolve(cliRoot, "..");
    const candidates = [
        resolve(repoRoot, "agent", "target", "release", "future-agent"),
        resolve(repoRoot, "agent", "target", "debug", "future-agent"),
    ];
    for (const candidate of candidates) {
        if (await canAccess(candidate, fsConstants.X_OK)) {
            return candidate;
        }
    }
    throw new Error("Future agent binary not found. Build it first with `cd future-os && make build-agent`, or set FUTURE_AGENT_BIN.");
}
function launchAgentPlist(options) {
    return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${escapeXml(options.label)}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${escapeXml(options.agentBinary)}</string>
    <string>--grpc-addr</string>
    <string>${escapeXml(options.grpcAddr)}</string>
  </array>
  <key>WorkingDirectory</key>
  <string>${escapeXml(homedir())}</string>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <false/>
  <key>StandardOutPath</key>
  <string>${escapeXml(options.stdoutPath)}</string>
  <key>StandardErrorPath</key>
  <string>${escapeXml(options.stderrPath)}</string>
</dict>
</plist>
`;
}
function resolveServiceName(defaultName, platformEnvName) {
    return process.env[platformEnvName] ?? process.env.FUTURE_AGENT_SERVICE_NAME ?? defaultName;
}
function getUserId() {
    if (typeof process.getuid !== "function") {
        throw new Error("Unable to resolve current user id for launchctl.");
    }
    return process.getuid();
}
function agentCommandPastTense(command) {
    if (command === "start") {
        return "Started";
    }
    if (command === "stop") {
        return "Stopped";
    }
    if (command === "restart") {
        return "Restarted";
    }
    return "Checked";
}
