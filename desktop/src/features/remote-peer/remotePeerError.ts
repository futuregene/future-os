import type { TFunction } from "i18next";

/**
 * The failure classes a remote link can report, in words the user can act on.
 *
 * The backend's transport errors are diagnostic strings: `Remote handshake
 * failed: no responders: no responders` names the request/reply lane of the
 * broker, and every sentence of it is aimed at someone reading a log. The class
 * decides which sentence the UI shows; the raw text stays with it as the
 * `title`, so a support session can still read the original.
 *
 * Matching is on the backend's own stable tokens (`pairing_link_expired`, …)
 * and on the NATS error prefixes the transport wraps, because those are the only
 * things the serialized `AppError` string exposes. `remote_peer::link` says
 * explicitly that user-facing text is the UI's job and that its tokens mirror
 * the phone's `PA*` codes — this module is that job.
 */
export type RemotePeerErrorKind
  = | "unreachable"
    | "timeout"
    | "pairing"
    | "pairingInUse"
    | "secureChannel"
    | "signedOut"
    | "refused";

const ERROR_KEYS: Record<RemotePeerErrorKind, string> = {
  unreachable: "errorUnreachable",
  timeout: "errorTimeout",
  pairing: "errorPairing",
  pairingInUse: "errorPairingInUse",
  secureChannel: "errorSecureChannel",
  signedOut: "errorSignedOut",
  refused: "errorRefused",
};

/**
 * Which sentence a raw failure maps to, or `null` when nothing matches.
 *
 * `null` rather than a catch-all sentence: an unrecognised failure is the one
 * case where the raw text is still the most useful thing on screen — it is what
 * a support session will ask for — so the caller keeps it instead of replacing
 * it with "something went wrong", which would leave the user with nothing.
 *
 * Order is load-bearing:
 * - pairing material first: an expired link's message also says `expired`, and
 *   re-pairing — not "update both apps" — is the only action there.
 * - connect failures before `timed out`: `Failed to connect to NATS: Timed out`
 *   never reached the host, so "it did not answer" is the honest reading and a
 *   retry is worth suggesting.
 * - `no responders` before everything transport-shaped: it is the exact shape
 *   of "the other desktop is not running its remote side", which is an
 *   actionable state, not a fault of our connection.
 */
export function remotePeerErrorKind(message: string): RemotePeerErrorKind | null {
  const text = message.trim();
  if (!text)
    return null;
  // A secure-channel refusal carries the host's *reason* in parentheses, and
  // the reason decides the action: an expired invitation is re-paired, a
  // disabled remote side is turned on, a busy host is retried — only an
  // unexplained mismatch means the two builds disagree. Reading the prefix
  // alone would send every one of them to "update both apps".
  const refusal = /remote_secure_channel_invalid\s*\(([^)]*)\)/i.exec(text);
  if (refusal) {
    const detail = refusal[1]!.toLowerCase();
    // A code another device already claimed is not an expired one: one invitation
    // pairs one device, and the remedy is a *separate* link, not regenerating the
    // same one. Both used to land on "pairing expired", which is the sentence this
    // report was filed against.
    if (detail === "invitation_already_used")
      return "pairingInUse";
    if (/invitation_|peer_mismatch|peer_unknown|identity|^peer$/.test(detail))
      return "pairing";
    if (detail === "disabled")
      return "unreachable";
    if (detail === "busy")
      return "timeout";
    return "secureChannel";
  }
  if (/invitation_already_used/i.test(text))
    return "pairingInUse";
  if (/pairing_link_|pairing_code|invalid_pairing|invalid_jwt|invitation_expired|pairing_signature_invalid|pairing_identity_mismatch|pairing_challenge_expired|challenge_expired|peer_mismatch|peer_not_paired/i.test(text))
    return "pairing";
  // The two builds never completed the encrypted handshake. That is a version
  // mismatch (or a corrupted pairing), not a network fault: retrying changes
  // nothing, and the only fix is updating both sides.
  if (/remote_secure_channel_invalid|confirmation_mismatch|e2ee_v2/i.test(text))
    return "secureChannel";
  if (/not signed in/i.test(text))
    return "signedOut";
  if (/authorization violation|remote_authorization|unauthorized/i.test(text))
    return "refused";
  if (/no responders/i.test(text))
    return "unreachable";
  if (/failed to connect to nats|nats_connect|connection refused|connection reset|network is unreachable|lookup host|io error|broken pipe|remote subscribe failed/i.test(text))
    return "unreachable";
  if (/timed out|timed_out|deadline has elapsed|timeout/i.test(text))
    return "timeout";
  // A command sent over a link that is no longer there. From the user's seat
  // that is the same fact as "could not reach it", and it is repaired the same
  // way. `nats_connection_exhausted` is the runtime's own record of a generation
  // whose socket closed for good (the retry that follows normally replaces it,
  // but a row read in between must not show the token).
  if (/peer_not_connected|not connected|nats_connection_exhausted/i.test(text))
    return "unreachable";
  return null;
}

/**
 * The sentence to show for a raw failure.
 *
 * Empty input answers an empty string rather than a sentence, so a caller that
 * renders the result unconditionally does not announce a failure that is not
 * there.
 */
export function remotePeerErrorText(
  message: string | null | undefined,
  t: TFunction<"remotePeer">,
): string {
  const raw = (message ?? "").trim();
  if (!raw)
    return "";
  const kind = remotePeerErrorKind(raw);
  return kind ? t(ERROR_KEYS[kind]) : raw;
}
