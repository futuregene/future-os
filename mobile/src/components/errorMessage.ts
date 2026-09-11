import { classifyAgentError } from "@future-os/thread-projection";
import { remoteErrorPresentation } from "../remote/errorPresentation";

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
  const presentation = remoteErrorPresentation(message);
  return t(presentation.messageKey, { code: presentation.supportCode });
}
