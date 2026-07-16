import { spawn } from "node:child_process";

/**
 * Launch a GUI process through Windows PowerShell's Start-Process.
 *
 * Start-Process uses the Windows graphical shell and creates an independent
 * process. This prevents Chrome from inheriting the agent/CLI stdout pipe,
 * which would otherwise keep the Rust shell tool waiting for EOF.
 */
export function launchWindowsDetached(executable: string, args: string[]): Promise<void> {
  const script = buildStartProcessScript(executable, args);
  const encoded = Buffer.from(script, "utf16le").toString("base64");
  const child = spawn("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-NoLogo",
    "-EncodedCommand",
    encoded,
  ], {
    stdio: "ignore",
    windowsHide: true,
  });

  // Wait only for Start-Process itself, not for Chrome. This both surfaces a
  // missing/failed PowerShell launch and guarantees that the browser creation
  // request has completed before browserStart begins polling CDP.
  return new Promise((resolve, reject) => {
    child.once("error", (error) => {
      reject(new Error(`Failed to launch browser through PowerShell: ${error.message}`));
    });
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      const detail = signal ? `signal ${signal}` : `exit code ${code ?? "unknown"}`;
      reject(new Error(`PowerShell failed to launch browser (${detail}).`));
    });
  });
}

export function buildStartProcessScript(executable: string, args: string[]): string {
  const argumentLine = args.map(quoteWindowsCommandLineArgument).join(" ");
  return [
    "$ErrorActionPreference = 'Stop'",
    `$process = Start-Process -FilePath ${quotePowerShellLiteral(executable)} -ArgumentList ${quotePowerShellLiteral(argumentLine)} -WindowStyle Normal -PassThru`,
    "if ($null -eq $process) { throw 'Start-Process did not return a process.' }",
  ].join("; ");
}

/** Quote one argv value using the CommandLineToArgvW parsing rules. */
export function quoteWindowsCommandLineArgument(value: string): string {
  if (value.length === 0) return '""';
  if (!/[\s"]/.test(value)) return value;

  let result = '"';
  let backslashes = 0;

  for (const char of value) {
    if (char === "\\") {
      backslashes += 1;
      continue;
    }

    if (char === '"') {
      result += "\\".repeat(backslashes * 2 + 1);
      result += '"';
    } else {
      result += "\\".repeat(backslashes);
      result += char;
    }
    backslashes = 0;
  }

  result += "\\".repeat(backslashes * 2);
  return `${result}"`;
}

function quotePowerShellLiteral(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}
