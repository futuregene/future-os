import { classifyAgentError } from "@future-os/thread-projection";

export type TranslateError = (key: string, opts?: Record<string, unknown>) => string;

/**
 * Raw agent run error → user-facing failure text, desktop parity: the shared
 * `classifyAgentError` maps the raw blob to the desktop `agent:failure.*` i18n
 * keys; mobile keeps the same keys under its single `failure` namespace.
 */
export function friendlyRunError(message: string | undefined, t: TranslateError): string {
  const { key, params } = classifyAgentError(message ?? "");
  return t(key.replace(/^agent:/, ""), params);
}

export function friendlyRunErrorTitle(message: string | undefined, t: TranslateError): string {
  const { key, params } = classifyAgentError(message ?? "");
  const titleKey = `${key.replace(/^agent:/, "")}Title`;
  const title = t(titleKey, params);
  return title === titleKey ? t("failure.runTitle") : title;
}

/** Convert backend/transport detail into a stable, actionable user message. */
export function friendlyError(message: string, t: TranslateError): string {
  const trimmed = message.trim();
  const code = /^(?:HTTP\s*)?(\d{3})$/.exec(trimmed)?.[1];
  if (code) {
    if (code === "401" || code === "403") return t("connection.errorPairing", { code: "PA002" });
    if (code === "404") return t("connection.errorNotFound", { code: "DT001" });
    if (code === "429") return t("connection.errorRateLimit", { code: "SV002" });
    if (code.startsWith("5")) return t("connection.errorServiceLater", { code: "SV001" });
  }
  if (/agent.*(offline|unavailable)|history is unavailable/i.test(trimmed)) {
    return t("connection.errorDeviceLater", { code: "LC003" });
  }
  if (/time-?out|timed out/i.test(trimmed)) {
    return t("connection.errorNetwork", { code: "NW002" });
  }
  if (/generation_unhealthy:protocol/i.test(trimmed)) {
    return t("connection.errorServiceSupport", { code: "PT001" });
  }
  if (/generation_unhealthy:subscription/i.test(trimmed)) {
    return t("connection.errorServiceLater", { code: "RT001" });
  }
  if (/remote_service_misconfigured|permissions_violation|authorization_violation/i.test(trimmed)) {
    return t("connection.errorServiceSupport", { code: "AU001" });
  }
  if (
    /network|unreachable|load failed|fetch failed|econn|not_connected|nats_|subscription_ended|connection (refused|reset)/i.test(
      trimmed,
    )
  ) {
    return t("connection.errorNetwork", { code: "NW001" });
  }
  if (
    /invalid_remote_credential|credentials_revoked|invalid_jwt|pairing_signature|confirmation_mismatch/i.test(
      trimmed,
    )
  ) {
    return t("connection.errorPairing", { code: "PA001" });
  }
  return t("connection.errorGeneric", { code: "LC999" });
}
