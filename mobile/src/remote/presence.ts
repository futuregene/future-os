import type { Presence } from "./types";

export const PRESENCE_STALE_AFTER_MS = 60_000;

/** Match the web client's desktop-presence rule. */
export function isDesktopOnline(presence: Presence | null, nowMs: number): boolean {
  if (!presence?.online || !presence.lastHeartbeatTs) return false;
  const ageMs = nowMs - presence.lastHeartbeatTs * 1000;
  return ageMs <= PRESENCE_STALE_AFTER_MS;
}
