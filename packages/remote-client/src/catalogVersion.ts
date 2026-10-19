import type { SnapshotVersion } from "./types";

/** Epoch is supplied by the authenticated handshake, never by a catalog packet. */
export class CatalogVersionGate {
  private epoch: string | undefined;
  private revisions = { sessions: -1, workspaces: -1 };

  authenticate(epoch: string | undefined): void {
    if (this.epoch === epoch) return;
    this.epoch = epoch;
    this.revisions = { sessions: -1, workspaces: -1 };
  }

  accept(domain: "sessions" | "workspaces", version?: SnapshotVersion, allowEqual = false): boolean {
    if (!this.epoch) return !version; // Old Desktop: retain request/arrival fencing.
    if (
      !version ||
      version.epoch !== this.epoch ||
      !Number.isSafeInteger(version.revision) ||
      version.revision < 0 ||
      (allowEqual ? version.revision < this.revisions[domain] : version.revision <= this.revisions[domain])
    )
      return false;
    this.revisions[domain] = version.revision;
    return true;
  }

  /**
   * The revision last applied for a domain, or `-1` before any snapshot. Used
   * to tell a presence-advertised revision that is merely current (nothing to
   * do) from one that is newer (a snapshot push was lost, so pull).
   */
  revision(domain: "sessions" | "workspaces"): number {
    return this.revisions[domain];
  }
}
