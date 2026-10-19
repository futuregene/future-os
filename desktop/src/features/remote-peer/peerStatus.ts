import type { RemotePeer } from "./remotePeerClient";

/**
 * How one paired host stands, in the same words the phone uses for the same
 * facts.
 *
 * Five states, because they are five different answers to "why are that
 * machine's conversations not in my list": its agent is up (`connected`), the
 * link is up but its agent is restarting (`preparing`), it is still being
 * reached (`connecting`), it could not be reached (`offline`), or the *user*
 * turned it off (`stopped`).
 *
 * `connecting` and `offline` are the pair that has to stay apart. The runtime
 * records an error against a host exactly when an attempt failed, so a host
 * with no error and no connection is one the app is reaching for the first time
 * — the state the launch pass puts a host in for its first second. Reading that
 * as "offline" would flash a failure that has not happened.
 */
export type PeerStatus = "connected" | "preparing" | "connecting" | "offline" | "stopped";

/**
 * One host's status, from the runtime's own facts.
 *
 * `stopped` is checked first: the user turning a machine off is a decision, and
 * it must not be reported as a fault. `error` is deliberately *not* consulted
 * for a stopped host: a deliberate disconnect leaves the last failure behind on
 * purpose.
 */
export function peerStatus(peer: RemotePeer): PeerStatus {
  if (peer.stopped)
    return "stopped";
  if (peer.connected)
    return peer.agentAvailable ? "connected" : "preparing";
  // A host that was reached and then failed says so; one that has never
  // answered is still being reached.
  return peer.error ? "offline" : "connecting";
}

/** i18n key for a status, under `remotePeer`. */
export function peerStatusKey(status: PeerStatus): string {
  if (status === "connected")
    return "statusConnected";
  if (status === "preparing")
    return "statusAgentUnavailable";
  if (status === "connecting")
    return "statusConnecting";
  if (status === "offline")
    return "statusOffline";
  return "statusDisconnected";
}

/** The `Badge` tone for a status: healthy, in-between, broken, or switched off. */
export function peerStatusTone(
  status: PeerStatus,
): "neutral" | "accent" | "warning" | "danger" {
  if (status === "connected")
    return "accent";
  if (status === "offline")
    return "danger";
  // A machine that is starting up, being reached, or whose agent is restarting
  // is not a failure: the amber says "not yet", the red says "no".
  if (status === "stopped")
    return "neutral";
  return "warning";
}

/**
 * Whether this machine is ours to drive: connected **and** not held elsewhere.
 *
 * The second half is what a status alone cannot say. A standby machine answers
 * every heartbeat — that is what lets its user take the machine back without
 * reconnecting — but it cannot read the host's conversations or prompt them: its
 * business commands come back `superseded`. The phone draws the same distinction
 * (`drivable` in `desktopStatus.ts`), and for the same reason: a green dot over a
 * list missing that machine's rows claims a machine works while nothing on it
 * does.
 */
export function peerDrivable(peer: RemotePeer): boolean {
  return peerStatus(peer) === "connected" && peer.control !== "standby";
}

/** Whether the machine answers but another paired device is driving it. */
export function peerHeld(peer: RemotePeer): boolean {
  return peer.control === "standby" && peerStatus(peer) === "connected";
}

/**
 * The word for one machine, which is not always its status.
 *
 * A machine that answers while another paired device drives it is `connected` on
 * the transport and unusable in fact: it has no conversations to show and
 * refuses every command. Calling it "connected" tells the reader everything is
 * fine, one line above a list missing exactly that machine — so the held case
 * gets its own word.
 */
export function peerLabelKey(peer: RemotePeer): string {
  return peerHeld(peer) ? "statusTakenOver" : peerStatusKey(peerStatus(peer));
}

/** The `Badge` tone for a machine, held included: amber, not green. */
export function peerLabelTone(
  peer: RemotePeer,
): "neutral" | "accent" | "warning" | "danger" {
  return peerHeld(peer) ? "warning" : peerStatusTone(peerStatus(peer));
}

/**
 * The name of the device holding the machine, when it declared one.
 *
 * `null` covers both "nobody holds it" and "the holder never said who it is";
 * the caller asks [`peerHeld`] and falls back to neutral wording instead of
 * inventing an identity.
 */
export function peerHolderName(peer: RemotePeer): string | null {
  const name = peer.holder?.name?.trim();
  return name || null;
}

/**
 * Whether every paired machine is connected — the rail dot's own condition.
 *
 * `all` is what the dot needs: a machine paired with three hosts is "connected"
 * only when all three answer, because the merged list is missing the ones that
 * do not, and that is exactly the state the reader has to be able to see. Only
 * `connected` counts, matching the phrase: a host whose agent is restarting has
 * no conversations to contribute either.
 */
export interface PeersConnected {
  all: boolean;
  any: boolean;
  connected: number;
  total: number;
}

export function peersConnected(peers: readonly RemotePeer[]): PeersConnected {
  const connected = peers.filter(peer => peerStatus(peer) === "connected").length;
  return {
    all: peers.length > 0 && connected === peers.length,
    any: connected > 0,
    connected,
    total: peers.length,
  };
}

/** The three states a set of machines can be drawn in, or `null` for none. */
export type PeersDotState = "up" | "partial" | "down";

/**
 * The dot's state for a set of machines, or `null` when there are none.
 *
 * `null` rather than "down": an unpaired machine has no client-role status to
 * show, and a red dot there would report a failure that does not exist.
 *
 * Green needs *every* machine up **and** drivable. A machine another paired
 * device holds answers every heartbeat and cannot be read from, so the merged
 * list is short of its conversations exactly as it is short of an offline
 * machine's — and a green dot over a short list is the one claim this dot exists
 * to avoid. The phone's set dot draws the same line (`drivable === total`);
 * "somewhere to work" is the rows' answer, not the header's.
 */
export function peersDotState(peers: readonly RemotePeer[]): PeersDotState | null {
  if (peers.length === 0)
    return null;
  const counts = peersConnected(peers);
  if (!counts.all)
    return counts.any ? "partial" : "down";
  return peers.every(peerDrivable) ? "up" : "partial";
}
