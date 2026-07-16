import { chmod, mkdir, writeFile } from "node:fs/promises";
import { homedir, platform as osPlatform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  DEFAULT_CHANNEL_LAUNCHD_LABEL,
  DEFAULT_CHANNEL_SYSTEMD_UNIT,
  DEFAULT_CHANNEL_WINDOWS_SERVICE,
} from "../constants.js";
import type { ChannelCommand, ServiceResult } from "../types.js";
import { assertExecutableFile, canAccess, fsConstants } from "../utils/files.js";
import { formatProcessOutput, runProcess } from "../utils/process.js";
import { escapeXml } from "../utils/string.js";

export async function channel(command: ChannelCommand): Promise<void> {
  const platform = osPlatform();
  if (platform === "darwin") {
    await runDarwinChannelCommand(command);
    return;
  }

  if (platform === "linux") {
    await runLinuxChannelCommand(command);
    return;
  }

  if (platform === "win32") {
    await runWindowsChannelCommand(command);
    return;
  }

  throw new Error(`Unsupported platform for channel service management: ${platform}`);
}

export function isChannelCommand(command: string | undefined): command is ChannelCommand {
  return command === "start" || command === "stop" || command === "restart" || command === "status";
}

async function runDarwinChannelCommand(command: ChannelCommand): Promise<void> {
  const label = resolveServiceName(DEFAULT_CHANNEL_LAUNCHD_LABEL, "FUTURE_CHANNEL_LAUNCHD_LABEL");
  const target = `gui/${getUserId()}/${label}`;

  if (command === "start") {
    await ensureDarwinLaunchAgent(label, target);
    await runServiceCommand("launchctl", ["kickstart", "-k", target], {
      successMessage: `Started Future channel service (${label}).`,
    });
    return;
  }

  if (command === "stop") {
    await runServiceCommand("launchctl", ["bootout", target], {
      failureHint: `Future channel launchd service is not loaded (${label}).`,
      successMessage: `Stopped Future channel service (${label}).`,
    });
    return;
  }

  if (command === "restart") {
    if (await isDarwinLaunchAgentLoaded(target)) {
      await runServiceCommand("launchctl", ["bootout", target]);
    }
    await ensureDarwinLaunchAgent(label, target);
    await runServiceCommand("launchctl", ["kickstart", "-k", target], {
      successMessage: `Restarted Future channel service (${label}).`,
    });
    return;
  }

  await runServiceCommand("launchctl", ["print", target], {
    failureHint: `Future channel launchd service is not loaded (${label}).`,
    printOutput: true,
  });
}

async function ensureDarwinLaunchAgent(label: string, target: string): Promise<void> {
  if (await isDarwinLaunchAgentLoaded(target)) {
    return;
  }

  const plistPath = await writeDarwinLaunchAgentPlist(label);
  const result = await runProcess("launchctl", ["bootstrap", `gui/${getUserId()}`, plistPath]);
  if (result.code !== 0) {
    const output = formatProcessOutput(result);
    throw new Error(
      `Failed to bootstrap Future channel launchd service (${label}).${output ? `\n${output}` : ""}`,
    );
  }
}

async function isDarwinLaunchAgentLoaded(target: string): Promise<boolean> {
  const result = await runProcess("launchctl", ["print", target]);
  return result.code === 0;
}

async function writeDarwinLaunchAgentPlist(label: string): Promise<string> {
  const channelBinary = await resolveChannelBinary();
  const launchAgentsDir = join(homedir(), "Library", "LaunchAgents");
  const logDir = join(homedir(), ".future", "channel", "logs");
  const plistPath = join(launchAgentsDir, `${label}.plist`);

  await mkdir(launchAgentsDir, { recursive: true });
  await mkdir(logDir, { recursive: true });
  await writeFile(
    plistPath,
    launchAgentPlist({
      label,
      channelBinary,
      stdoutPath: join(logDir, "channel.out.log"),
      stderrPath: join(logDir, "channel.err.log"),
    }),
    { mode: 0o644 },
  );
  await chmod(plistPath, 0o644);
  return plistPath;
}

async function runLinuxChannelCommand(command: ChannelCommand): Promise<void> {
  const unit = resolveServiceName(DEFAULT_CHANNEL_SYSTEMD_UNIT, "FUTURE_CHANNEL_SYSTEMD_UNIT");

  if (command === "status") {
    await runServiceCommand("systemctl", ["--user", "status", "--no-pager", unit], {
      failureHint: `Future channel systemd user service is not active or not installed (${unit}).`,
      printOutput: true,
    });
    return;
  }

  await runServiceCommand("systemctl", ["--user", command, unit], {
    successMessage: `${channelCommandPastTense(command)} Future channel service (${unit}).`,
  });
}

async function runWindowsChannelCommand(command: ChannelCommand): Promise<void> {
  const service = resolveServiceName(DEFAULT_CHANNEL_WINDOWS_SERVICE, "FUTURE_CHANNEL_WINDOWS_SERVICE");

  if (command === "status") {
    await runServiceCommand("sc.exe", ["query", service], {
      failureHint: `Future channel Windows service is not installed (${service}).`,
      printOutput: true,
    });
    return;
  }

  if (command === "restart") {
    await runWindowsChannelCommand("stop");
    await runWindowsChannelCommand("start");
    return;
  }

  const action = command === "start" ? "start" : "stop";
  await runServiceCommand("sc.exe", [action, service], {
    successMessage: `${channelCommandPastTense(command)} Future channel service (${service}).`,
  });
}

async function runServiceCommand(
  command: string,
  args: string[],
  options: {
    successMessage?: string;
    failureHint?: string;
    printOutput?: boolean;
  } = {},
): Promise<ServiceResult> {
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
  } else if (options.successMessage) {
    console.log(options.successMessage);
  }

  return result;
}

async function resolveChannelBinary(): Promise<string> {
  const override = process.env.FUTURE_CHANNEL_BIN;
  if (override) {
    await assertExecutableFile(override, "FUTURE_CHANNEL_BIN");
    return override;
  }

  const currentFile = fileURLToPath(import.meta.url);
  const cliRoot = resolve(dirname(currentFile), "..", "..");
  const repoRoot = resolve(cliRoot, "..");
  const candidates = [
    resolve(repoRoot, "channels", "target", "release", "future-channel"),
    resolve(repoRoot, "channels", "target", "debug", "future-channel"),
  ];

  for (const candidate of candidates) {
    if (await canAccess(candidate, fsConstants.X_OK)) {
      return candidate;
    }
  }

  throw new Error(
    "Future channel binary not found. Build it first with `cd future-os && make build-channels`, or set FUTURE_CHANNEL_BIN.",
  );
}

function launchAgentPlist(options: {
  label: string;
  channelBinary: string;
  stdoutPath: string;
  stderrPath: string;
}): string {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${escapeXml(options.label)}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${escapeXml(options.channelBinary)}</string>
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

function resolveServiceName(defaultName: string, platformEnvName: string): string {
  return process.env[platformEnvName] ?? process.env.FUTURE_CHANNEL_SERVICE_NAME ?? defaultName;
}

function getUserId(): number {
  if (typeof process.getuid !== "function") {
    throw new Error("Unable to resolve current user id for launchctl.");
  }
  return process.getuid();
}

function channelCommandPastTense(command: ChannelCommand): string {
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
