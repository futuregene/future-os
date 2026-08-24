import { useEffect, useState } from "react";
import { isMacOS, isWindows } from "../../lib/platform";
import { invokeCommand } from "../tauri/invoke";

interface WindowsSandboxProbeResult {
  available: boolean;
  code: string;
  rolloutEnabled: boolean;
}

export interface SandboxAvailability {
  available: boolean;
  resolved: boolean;
}

let windowsProbe: Promise<boolean> | null = null;

export function windowsSandboxAvailable(result: WindowsSandboxProbeResult): boolean {
  return result.rolloutEnabled && result.available;
}

async function probeWindowsRollout(): Promise<boolean> {
  try {
    const result = await invokeCommand<WindowsSandboxProbeResult>("probe_windows_sandbox");
    return windowsSandboxAvailable(result);
  }
  catch {
    // Agent startup/probe failures fail closed. A later app start probes again.
    return false;
  }
}

function initialAvailability(): SandboxAvailability {
  if (isMacOS)
    return { available: true, resolved: true };
  if (!isWindows)
    return { available: false, resolved: true };
  return { available: false, resolved: false };
}

/**
 * Product-facing sandbox availability. Windows requires both the hidden W7
 * rollout gate and a successful native host probe; the promise is shared so
 * Settings and Composer do not independently exercise the native probe.
 */
export function useSandboxAvailability(): SandboxAvailability {
  const [availability, setAvailability] = useState(initialAvailability);

  useEffect(() => {
    if (!isWindows)
      return;
    windowsProbe ??= probeWindowsRollout();
    let current = true;
    void windowsProbe.then((available) => {
      if (current)
        setAvailability({ available, resolved: true });
    });
    return () => {
      current = false;
    };
  }, []);

  return availability;
}
