export type TranslateError = (key: string, opts?: Record<string, unknown>) => string;

/** Convert backend/transport detail into a stable, actionable user message. */
export function friendlyError(message: string, t: TranslateError): string {
  const trimmed = message.trim();
  const code = /^(?:HTTP\s*)?(\d{3})$/.exec(trimmed)?.[1];
  if (code) {
    if (code === "401" || code === "403") return t("connection.errorAuth");
    if (code === "404") return t("connection.errorNotFound");
    if (code === "429") return t("connection.errorRateLimit");
    if (code.startsWith("5")) return t("connection.errorService");
  }
  if (/agent.*(offline|unavailable)|history is unavailable/i.test(trimmed)) {
    return t("connection.errorAgentOffline");
  }
  if (/time-?out|timed out/i.test(trimmed)) return t("connection.errorTimeout");
  if (
    /network|unreachable|load failed|fetch failed|econn|not_connected|nats_|subscription_ended|connection (refused|reset)/i.test(
      trimmed,
    )
  ) {
    return t("connection.errorNetwork");
  }
  if (
    /invalid_remote_credential|credentials_revoked|invalid_jwt|pairing_signature|confirmation_mismatch/i.test(
      trimmed,
    )
  ) {
    return t("connection.errorAuth");
  }
  return t("connection.errorGeneric");
}
