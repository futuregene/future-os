/**
 * A version gate for host-pushed catalog snapshots.
 *
 * Mobile twin: `mobile/src/remote/catalogVersion.ts`. The two are intentionally
 * kept close so the same correctness arguments apply on both clients.
 *
 * Epoch comes from the handshake (or, on desktop, the first presence that carries
 * `catalogVersion.epoch`). Once authenticated, only snapshots whose epoch matches
 * and whose revision is newer (or equal when explicitly allowed) are accepted.
 */

export interface SnapshotVersion {
  /** The handshake epoch; resets on every re-pair/reconnect. */
  epoch: string;
  /** Monotonically increasing revision within an epoch. */
  revision: number;
}

export class CatalogVersionGate {
  private epoch: string | null = null;
  private revisions = new Map<string, number>();

  /** True once an epoch has been set (handshake confirmed). */
  get authenticated(): boolean {
    return this.epoch !== null;
  }

  /** Set the epoch from the handshake / presence; resets applied revisions. */
  authenticate(epoch: string): void {
    this.epoch = epoch;
    this.revisions.clear();
  }

  /**
   * Decide whether a pushed/polled `version` may be applied for `domain`.
   *
   * @param domain - which catalog ("sessions" | "workspaces")
   * @param version - the snapshot version, or undefined for old hosts
   * @param allowEqual - whether the same revision may be accepted. Explicit
   *   reads (list_sessions / list_workspaces) pass true because they confirm an
   *   optimistic update; pushed snapshots pass false.
   * @returns whether the snapshot should be applied
   */
  accept(domain: string, version?: SnapshotVersion, allowEqual = false): boolean {
    if (this.epoch === null) {
      // Old host that does not publish versions. Accept only if it also does
      // not claim a version — a version without an epoch is a protocol violation.
      return version === undefined;
    }
    if (version === undefined) {
      // A new host must always publish a version with its snapshots; absence is
      // a protocol violation.
      return false;
    }
    if (version.epoch !== this.epoch) {
      return false;
    }
    const previous = this.revisions.get(domain) ?? -1;
    if (version.revision < previous) {
      return false;
    }
    if (version.revision === previous && !allowEqual) {
      return false;
    }
    this.revisions.set(domain, version.revision);
    return true;
  }

  /** The last accepted revision for a domain, or -1 if none. */
  revision(domain: string): number {
    return this.revisions.get(domain) ?? -1;
  }
}
