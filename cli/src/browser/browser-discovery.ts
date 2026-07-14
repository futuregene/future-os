/**
 * Browser discovery: find installed Chrome/Edge/Chromium.
 * Extracted from the existing findBrowserLauncher() in browser-tools.ts.
 */
import { existsSync } from "node:fs";
import { platform } from "node:os";
import { join } from "node:path";
export type ChromiumBrowserKind = "chrome" | "edge" | "chromium";

export interface BrowserExecutable {
  kind: ChromiumBrowserKind;
  executablePath: string;
  source: "env" | "known-path" | "path";
}

/**
 * Find installed Chromium-based browsers in priority order:
 * Chrome → Edge → Chromium
 *
 * Accepts an optional user-specified path (from executablePath arg).
 */
export function findBrowser(executablePath?: string): BrowserExecutable | null {
  if (executablePath) {
    const kind = inferKind(executablePath);
    return { kind, executablePath, source: "path" };
  }

  if (platform() === "darwin") {
    return findMacOSBrowser();
  }
  if (platform() === "win32") {
    return findWindowsBrowser();
  }
  return findLinuxBrowser();
}

function findMacOSBrowser(): BrowserExecutable | null {
  const candidates: Array<{ kind: "chrome" | "edge" | "chromium"; path: string }> = [
    { kind: "chrome", path: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" },
    { kind: "edge", path: "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" },
    { kind: "chromium", path: "/Applications/Chromium.app/Contents/MacOS/Chromium" },
  ];
  for (const { kind, path } of candidates) {
    if (existsSync(path)) return { kind, executablePath: path, source: "known-path" };
  }
  return null;
}

function findWindowsBrowser(): BrowserExecutable | null {
  const local = process.env["LOCALAPPDATA"];
  const prog = process.env["PROGRAMFILES"];
  const progX86 = process.env["PROGRAMFILES(X86)"];

  const candidates: Array<{ kind: "chrome" | "edge"; path: string }> = [
    { kind: "chrome", path: local ? join(local, "Google", "Chrome", "Application", "chrome.exe") : "" },
    { kind: "chrome", path: prog ? join(prog, "Google", "Chrome", "Application", "chrome.exe") : "" },
    { kind: "edge", path: progX86 ? join(progX86, "Microsoft", "Edge", "Application", "msedge.exe") : "" },
    { kind: "edge", path: prog ? join(prog, "Microsoft", "Edge", "Application", "msedge.exe") : "" },
  ];

  for (const c of candidates) {
    if (c.path && existsSync(c.path)) return { kind: c.kind, executablePath: c.path, source: "known-path" };
  }
  return null;
}

function findLinuxBrowser(): BrowserExecutable | null {
  // CHROME_PATH env var
  const envPath = process.env["CHROME_PATH"];
  if (envPath && existsSync(envPath)) {
    return { kind: inferKind(envPath), executablePath: envPath, source: "env" };
  }
  // Known paths
  const candidates: Array<{ kind: "chrome" | "chromium"; path: string }> = [
    { kind: "chrome", path: "/usr/bin/google-chrome" },
    { kind: "chromium", path: "/usr/bin/chromium-browser" },
    { kind: "chromium", path: "/usr/bin/chromium" },
  ];
  for (const { kind, path } of candidates) {
    if (existsSync(path)) return { kind, executablePath: path, source: "known-path" };
  }
  return null;
}

function inferKind(path: string): ChromiumBrowserKind {
  const lower = path.toLowerCase();
  if (lower.includes("edge") || lower.includes("msedge")) return "edge";
  if (lower.includes("chrome")) return "chrome";
  if (lower.includes("chromium")) return "chromium";
  return "chrome";
}
