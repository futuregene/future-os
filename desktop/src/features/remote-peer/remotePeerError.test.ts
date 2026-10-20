import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import i18n from "../../i18n";
import en from "../../i18n/locales/en/remotePeer.json";
import { remotePeerErrorKind, remotePeerErrorText } from "./remotePeerError";

/**
 * The connection failures this page can show, in the reader's words.
 *
 * Each case is a real backend string: the transport's are what
 * `remote_peer::session` formats, and the secure-channel details are the ones
 * `remote::secure` appends. The point of the mapping is that none of them reach
 * the screen as written.
 */
const t = i18n.getFixedT("en", "remotePeer") as TFunction<"remotePeer">;

describe("remotePeerErrorText", () => {
  /**
   * The failure the user actually reported: a desktop with its remote side off
   * answers nothing, and the broker's "no responders" is all the backend can
   * say. The double "no responders: no responders" is the library's own
   * wrapping, and it is exactly the kind of text this mapping removes.
   */
  it("turns a silent desktop into an actionable sentence", () => {
    const raw = "Remote handshake failed: no responders: no responders";

    expect(remotePeerErrorKind(raw)).toBe("unreachable");
    expect(remotePeerErrorText(raw, t)).toBe(en.errorUnreachable);
    expect(remotePeerErrorText(raw, t)).not.toMatch(/no responders|handshake/i);
  });

  /**
   * A connect that timed out never reached the host. "It did not answer" and a
   * retry are the honest reading — not the protocol-mismatch sentence, which is
   * what the word "handshake" would otherwise suggest.
   */
  it("does not report a connect timeout as a protocol problem", () => {
    expect(remotePeerErrorKind("Failed to connect to NATS: Timed out")).toBe("unreachable");
  });

  it("keeps a request timeout its own class", () => {
    const raw = "Remote handshake failed: request timed out: deadline has elapsed";

    expect(remotePeerErrorKind(raw)).toBe("timeout");
    expect(remotePeerErrorText(raw, t)).toBe(en.errorTimeout);
  });

  /**
   * A generation whose socket closed for good is recorded by the runtime as
   * `nats_connection_exhausted` until the retry replaces it. From the user's seat
   * it is an unreachable host, not a token on the row.
   */
  it("reads an exhausted connection as unreachable", () => {
    expect(remotePeerErrorKind("nats_connection_exhausted")).toBe("unreachable");
  });

  /**
   * Pairing and identity refusals all end in a new link, whatever the host's
   * detail says — including the ones that arrive under the secure-channel
   * prefix, whose reason is in parentheses.
   */
  it.each([
    "pairing_link_expired",
    "invitation_expired",
    "remote_secure_channel_invalid (invitation_expired)",
    "remote_secure_channel_invalid (peer_mismatch)",
    "challenge_expired",
    "peer_not_paired",
  ])("asks for a new pairing for %s", (raw) => {
    expect(remotePeerErrorKind(raw)).toBe("pairing");
    expect(remotePeerErrorText(raw, t)).toBe(en.errorPairing);
  });

  /** A disabled remote side is a desktop to wake, not a pairing to redo. */
  it("reads a disabled host as unreachable", () => {
    expect(remotePeerErrorKind("remote_secure_channel_invalid (disabled)")).toBe("unreachable");
  });

  /** An unexplained refusal is the one that means the builds disagree. */
  it("treats an unmatched secure-channel refusal as a version problem", () => {
    expect(remotePeerErrorKind("remote_secure_channel_invalid (open_undecryptable)")).toBe("secureChannel");
    expect(remotePeerErrorText("remote_secure_channel_invalid (open_undecryptable)", t))
      .toBe(en.errorSecureChannel);
  });

  it("names an account and a refusal where those are the cause", () => {
    expect(remotePeerErrorKind("Not signed in to FutureOS.")).toBe("signedOut");
    expect(remotePeerErrorKind("Authorization Violation")).toBe("refused");
  });

  /**
   * An unknown failure keeps the backend's words: it is the one case where the
   * raw text is what a support session needs, and replacing it with a generic
   * sentence would hide the only lead there is.
   */
  it("leaves a failure it does not recognise untouched", () => {
    const raw = "some future backend failure";

    expect(remotePeerErrorKind(raw)).toBeNull();
    expect(remotePeerErrorText(raw, t)).toBe(raw);
  });

  it("does not announce a failure when there is none", () => {
    expect(remotePeerErrorText(null, t)).toBe("");
    expect(remotePeerErrorText("   ", t)).toBe("");
  });
});
