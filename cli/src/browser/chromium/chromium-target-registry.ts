/**
 * Target Session Registry — maps CDP targetId ↔ sessionId.
 *
 * Provides unified cleanup via detachBySessionId / detachByTargetId.
 * Both Target.detachedFromTarget (has sessionId) and
 * Target.targetDestroyed (has targetId) may fire for the same target —
 * the detach methods are idempotent (return undefined on second call).
 */
export interface AttachedTarget {
  targetId: string;
  sessionId: string;
  type: string; // "page"
}

export class TargetSessionRegistry {
  private byTargetId = new Map<string, AttachedTarget>();
  private bySessionId = new Map<string, AttachedTarget>();

  add(target: AttachedTarget): void {
    this.byTargetId.set(target.targetId, target);
    this.bySessionId.set(target.sessionId, target);
  }

  /** Remove by sessionId. Returns undefined if already removed. */
  detachBySessionId(sessionId: string): AttachedTarget | undefined {
    const target = this.bySessionId.get(sessionId);
    if (!target) return undefined;
    this.bySessionId.delete(sessionId);
    this.byTargetId.delete(target.targetId);
    return target;
  }

  /** Remove by targetId. Returns undefined if already removed. */
  detachByTargetId(targetId: string): AttachedTarget | undefined {
    const target = this.byTargetId.get(targetId);
    if (!target) return undefined;
    this.byTargetId.delete(targetId);
    this.bySessionId.delete(target.sessionId);
    return target;
  }

  getByTargetId(targetId: string): AttachedTarget | undefined {
    return this.byTargetId.get(targetId);
  }

  getBySessionId(sessionId: string): AttachedTarget | undefined {
    return this.bySessionId.get(sessionId);
  }

  getAttachedPageIds(): string[] {
    return Array.from(this.byTargetId.keys());
  }

  clear(): void {
    this.byTargetId.clear();
    this.bySessionId.clear();
  }
}
