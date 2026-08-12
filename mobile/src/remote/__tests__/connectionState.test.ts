import {
  backoffDelayMs,
  classifyError,
  MAX_BACKOFF_MS,
  transition,
  type ConnectionEffect,
  type ConnectionState,
  type LifecycleEvent,
} from "../connectionState";

const ALL_STATES: ConnectionState[] = [
  "connecting",
  "ready",
  "reconnecting",
  "refreshing",
  "revoked",
  "unpaired",
];

const TERMINAL: ConnectionState[] = ["revoked", "unpaired"];

function runTable(): Record<string, ConnectionState> {
  const matrix: LifecycleEvent[] = [
    { type: "open_started" },
    { type: "open_failed", error: new Error("boom") },
    { type: "ready" },
    { type: "transport_disconnect" },
    { type: "auth_failed" },
    { type: "revoked" },
    { type: "unpair" },
  ];
  const table: Record<string, ConnectionState> = {};
  for (const state of ALL_STATES) {
    for (const event of matrix) {
      table[`${state}+${event.type}`] = transition(state, event).next;
    }
  }
  return table;
}

describe("connectionState FSM", () => {
  test("connecting/open_started is a no-op (oldest attempt wins)", () => {
    expect(transition("connecting", { type: "open_started" })).toEqual({
      next: "connecting",
      effects: [],
    });
  });

  test("connecting/ready → ready with no effects", () => {
    expect(transition("connecting", { type: "ready" })).toEqual({ next: "ready", effects: [] });
  });

  test("ready/transport_disconnect never arms a retry (NATS reconnects internally)", () => {
    // Only a genuinely-dead attempt (open_failed) may arm the backoff timer.
    // A disconnect while ready keeps the same connection — NATS's own status
    // loop reconnects it and re-emits ready. Arming a second timer here is
    // exactly the H4 bug (close() → disconnected → 1s timer).
    for (const event of [
      { type: "transport_disconnect" },
      { type: "auth_failed" },
      { type: "revoked" },
      { type: "unpair" },
      { type: "open_started" },
      { type: "ready" },
    ] as LifecycleEvent[]) {
      const action = transition("ready", event);
      expect(action.effects.some(e => e.type === "schedule_reconnect")).toBe(false);
    }
  });

  test("a failed attempt (open_failed) always arms a retry", () => {
    for (const state of ["connecting", "reconnecting", "refreshing"] as const) {
      const action = transition(state, { type: "open_failed", error: new Error("x") });
      expect(action.next).toBe("reconnecting");
      expect(action.effects).toEqual([{ type: "schedule_reconnect" }]);
    }
  });

  test("connecting/open_failed → reconnecting WITH a retry timer", () => {
    const action = transition("connecting", { type: "open_failed", error: new Error("x") });
    expect(action.next).toBe("reconnecting");
    expect(action.effects).toEqual([{ type: "schedule_reconnect" }]);
  });

  test("auth_failed → refreshing (token rotation), from ready or connecting", () => {
    for (const state of ["ready", "connecting", "reconnecting"] as const) {
      const action = transition(state, { type: "auth_failed" });
      expect(action.next).toBe("refreshing");
      expect(action.effects).toEqual([{ type: "begin_token_refresh" }]);
    }
  });

  test("revoked → revoked terminal, disposes the connection, never schedules", () => {
    for (const state of ["ready", "connecting", "reconnecting", "refreshing"] as const) {
      const action = transition(state, { type: "revoked" });
      expect(action.next).toBe("revoked");
      expect(action.effects).toEqual([{ type: "dispose_connection", reason: "revoked" }]);
    }
  });

  test("unpair → unpaired terminal, disposes + enters unpaired", () => {
    for (const state of ["ready", "connecting", "reconnecting", "refreshing"] as const) {
      const action = transition(state, { type: "unpair" });
      expect(action.next).toBe("unpaired");
      expect(action.effects).toEqual([
        { type: "dispose_connection", reason: "unpair" },
        { type: "enter_unpaired" },
      ]);
    }
  });

  test("terminal states ignore every event — except unpaired/open_started", () => {
    // revoked is a hard stop; unpaired only exits via a fresh open (a new
    // RemoteClient starts its first connect from unpaired).
    const matrix: LifecycleEvent[] = [
      { type: "open_started" },
      { type: "open_failed", error: new Error("x") },
      { type: "ready" },
      { type: "transport_disconnect" },
      { type: "auth_failed" },
      { type: "revoked" },
      { type: "unpair" },
    ];
    for (const terminal of TERMINAL) {
      for (const event of matrix) {
        const action = transition(terminal, event);
        expect(action.effects).toEqual([]);
        if (terminal === "unpaired" && event.type === "open_started") {
          expect(action.next).toBe("connecting");
        } else {
          expect(action.next).toBe(terminal);
        }
      }
    }
  });

  test("the full state×event table stays in revoked; unpaired only exits on open_started", () => {
    const table = runTable();
    for (const key of Object.keys(table)) {
      const [from, event] = key.split("+");
      if (from === "revoked") {
        expect(table[key]).toBe("revoked");
      }
      if (from === "unpaired") {
        expect(table[key]).toBe(event === "open_started" ? "connecting" : "unpaired");
      }
    }
  });

  test("H4 regression: a ready↔disconnect cycle never re-arms the retry timer", () => {
    // The audit's H4: close() broadcast "disconnected" mid-refresh, arming a
    // 1s timer while the previous open was still in flight — a reconnect storm
    // that only cleared once the network got fast enough. The FSM must never
    // arm a timer from a ready-state disconnect (NATS reconnects the same
    // connection internally); only a genuinely-dead open attempt may.
    let state: ConnectionState = "connecting";
    const transitions: ConnectionEffect[][] = [];
    for (let i = 0; i < 10; i += 1) {
      const connected = transition(state, { type: "ready" });
      transitions.push(connected.effects);
      state = connected.next;
      expect(state).toBe("ready");

      // The transport blips — NATS reconnects on its own; no timer may arm.
      const blipped = transition(state, { type: "transport_disconnect" });
      transitions.push(blipped.effects);
      state = blipped.next;
      expect(state).toBe("reconnecting");
      expect(blipped.effects).toEqual([]);
    }
    // Across 10 blips, exactly zero retry timers were armed.
    const armed = transitions.flat().filter(effect => effect.type === "schedule_reconnect");
    expect(armed).toEqual([]);
  });

  test("H4 regression: a slow open that dies arms exactly one timer", () => {
    // A connect attempt that actually fails (open > backoff interval, then
    // dead) must arm the backoff timer exactly once — not once per
    // disconnect-status — so weak networks back off instead of storming.
    let state: ConnectionState = "connecting";
    const failed = transition(state, { type: "open_failed", error: new Error("timeout") });
    expect(failed.next).toBe("reconnecting");
    expect(failed.effects.filter(e => e.type === "schedule_reconnect")).toHaveLength(1);
    state = failed.next;

    // While recovering, another disconnect status is absorbed (no re-arm).
    const absorbed = transition(state, { type: "transport_disconnect" });
    expect(absorbed.next).toBe("reconnecting");
    expect(absorbed.effects).toEqual([]);
  });
});

describe("backoffDelayMs", () => {
  test("caps at MAX_BACKOFF_MS", () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      expect(backoffDelayMs(attempt)).toBeLessThanOrEqual(MAX_BACKOFF_MS);
    }
  });
  test("grows with attempt (deterministic random)", () => {
    const r0 = () => 0;
    const r1 = () => 1;
    expect(backoffDelayMs(0, r0)).toBe(1000);
    expect(backoffDelayMs(1, r0)).toBe(2000);
    expect(backoffDelayMs(2, r0)).toBe(4000);
    expect(backoffDelayMs(4, r0)).toBe(16000);
    expect(backoffDelayMs(5, r0)).toBe(MAX_BACKOFF_MS); // 32000 → capped
    expect(backoffDelayMs(6, r0)).toBe(MAX_BACKOFF_MS);
    expect(backoffDelayMs(5, r1)).toBe(MAX_BACKOFF_MS); // 38400 → capped
  });
});

describe("classifyError", () => {
  test("terminal auth errors → authTerminal", () => {
    expect(classifyError(new Error("invalid_remote_credential"))).toBe("authTerminal");
    expect(classifyError(new Error("server replied 401"))).toBe("authTerminal");
    expect(classifyError(new Error("credentials_revoked"))).toBe("authTerminal");
    expect(classifyError(new Error("invalid_jwt"))).toBe("authTerminal");
  });
  test("refreshable auth errors → auth", () => {
    expect(classifyError(new Error("pairing_signature_invalid"))).toBe("auth");
    expect(classifyError(new Error("pairing_confirmation_mismatch"))).toBe("auth");
  });
  test("everything else → transport", () => {
    expect(classifyError(new Error("nats_connect_failed"))).toBe("transport");
    expect(classifyError(new Error("ETIMEDOUT"))).toBe("transport");
    expect(classifyError("boom")).toBe("transport");
  });
});
