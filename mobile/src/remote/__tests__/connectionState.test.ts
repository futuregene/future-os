import {
  backoffDelayMs,
  classifyError,
  MAX_BACKOFF_MS,
  transition,
  type ConnectionEffect,
  type ConnectionState,
  type LifecycleEvent,
} from "../connectionState";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
import type { RemoteCredentials } from "../types";
import { ErrorCode, NatsError } from "nats.ws";

const ALL_STATES: ConnectionState[] = [
  "connecting",
  "ready",
  "reconnecting",
  "refreshing",
  "failed",
  "revoked",
  "unpaired",
];

const TERMINAL: ConnectionState[] = ["failed", "revoked", "unpaired"];

function runTable(): Record<string, ConnectionState> {
  const matrix: LifecycleEvent[] = [
    { type: "open_started" },
    { type: "open_failed", error: new Error("boom") },
    { type: "ready" },
    { type: "transport_disconnect" },
    { type: "auth_failed" },
    { type: "fatal", error: new Error("fatal") },
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
      { type: "fatal", error: new Error("fatal") },
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

  test("fatal NATS configuration errors enter a failed terminal state", () => {
    for (const state of ["ready", "connecting", "reconnecting", "refreshing"] as const) {
      const error = new Error("remote_service_misconfigured");
      const action = transition(state, { type: "fatal", error });
      expect(action.next).toBe("failed");
      expect(action.effects).toEqual([{ type: "dispose_connection", reason: "fatal" }]);
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

  test("unrecognized event types degrade to a no-op instead of falling through", () => {
    // A forward-compat guard: a new LifecycleEvent variant that an older FSM
    // build doesn't know about must be absorbed (state + no effects), never
    // accidentally dropped into another transition arm.
    const action = transition("connecting", {
      type: "unrecognized",
    } as unknown as LifecycleEvent);
    expect(action).toEqual({ next: "connecting", effects: [] });
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
      { type: "fatal", error: new Error("fatal") },
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
      if (from === "revoked" || from === "failed") {
        expect(table[key]).toBe(from);
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

    // While reconnecting, another disconnect status is absorbed (no re-arm).
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
  test("service permission errors → fatal", () => {
    expect(classifyError(new Error("PERMISSIONS_VIOLATION"))).toBe("fatal");
    expect(classifyError(new Error("remote_service_misconfigured"))).toBe("fatal");
  });
  test("everything else → transport", () => {
    expect(classifyError(new Error("nats_connect_failed"))).toBe("transport");
    expect(classifyError(new Error("ETIMEDOUT"))).toBe("transport");
    expect(classifyError("boom")).toBe("transport");
  });
});

function recoveryClient(): {
  client: RemoteClient;
  callbacks: jest.Mocked<RemoteClientCallbacks>;
} {
  const credentials: RemoteCredentials = {
    pairId: "pair_1",
    deviceId: "device_1",
    seed: "unused",
    userJwt: "unused",
    refreshToken: "unused",
    natsWsUrl: "wss://nats.example",
    tokenUrl: "https://example.com/token",
    expectedDesktopId: "desktop_1",
    expectedDesktopPublicKey: "UDESKTOP",
  };
  const callbacks = {
    onCredentials: jest.fn(),
    onEvent: jest.fn(),
    onEventDecodeFailure: jest.fn(),
    onPresence: jest.fn(),
    onSessions: jest.fn(),
    onWorkspaces: jest.fn(),
    onFeatures: jest.fn(),
    onConnectionState: jest.fn(),
    onReconnected: jest.fn(),
    onError: jest.fn(),
  } as jest.Mocked<RemoteClientCallbacks>;
  return { client: new RemoteClient(credentials, callbacks), callbacks };
}

describe("RemoteClient terminal iterator recovery", () => {
  test("a permission status becomes a terminal service failure", async () => {
    const { client } = recoveryClient();
    const recovery = jest.fn();
    const testClient = client as unknown as {
      watchStatus(connection: unknown, generation: number): void;
      handleFailure(error: unknown): void;
    };
    testClient.handleFailure = recovery;
    async function* statuses() {
      yield { type: "error", data: ErrorCode.PermissionsViolation };
    }
    testClient.watchStatus({ status: statuses }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(recovery).toHaveBeenCalledWith(
      expect.objectContaining({ message: expect.stringContaining("remote_service_misconfigured") }),
    );
    expect(recovery).toHaveBeenCalledTimes(1);
  });

  test("a protocol status fails its generation once without exhaustion fallback", async () => {
    const { client } = recoveryClient();
    const failGeneration = jest.fn();
    const testClient = client as unknown as {
      watchStatus(connection: unknown, generation: number): void;
      failGeneration(error: unknown, generation: number): void;
    };
    testClient.failGeneration = failGeneration;
    async function* statuses() {
      yield { type: "error", data: ErrorCode.ProtocolError };
    }
    testClient.watchStatus({ status: statuses }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(failGeneration).toHaveBeenCalledTimes(1);
    expect(failGeneration).toHaveBeenCalledWith(
      expect.objectContaining({ message: expect.stringContaining("nats_protocol_error") }),
      0,
    );
  });

  test("a throwing NATS status iterator enters the outer recovery path", async () => {
    const { client } = recoveryClient();
    const recovery = jest.fn();
    const testClient = client as unknown as {
      watchStatus(connection: unknown, generation: number): void;
      handleFailure(error: unknown): void;
    };
    testClient.handleFailure = recovery;
    async function* statuses(): AsyncGenerator<never> {
      throw new Error("status iterator failed");
    }
    testClient.watchStatus({ status: statuses }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(recovery).toHaveBeenCalledWith(expect.any(Error));
  });

  test("a throwing event subscription cannot die as an unhandled task", async () => {
    const { client } = recoveryClient();
    const recovery = jest.fn();
    const testClient = client as unknown as {
      subscribeEvents(connection: unknown, generation: number): void;
      handleFailure(error: unknown): void;
    };
    testClient.handleFailure = recovery;
    async function* events(): AsyncGenerator<never> {
      throw new Error("event iterator failed");
    }
    testClient.subscribeEvents({ subscribe: events }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(recovery).toHaveBeenCalledWith(expect.any(Error));
  });

  test("a malformed live event requests immediate session reconciliation", async () => {
    const { client, callbacks } = recoveryClient();
    const testClient = client as unknown as {
      subscribeEvents(connection: unknown, generation: number): void;
    };
    async function* events() {
      yield {
        subject: "p.pair_1.evt.session_1",
        data: new TextEncoder().encode("{not-json"),
      };
      await new Promise(() => {});
    }
    testClient.subscribeEvents({ subscribe: () => events() }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(callbacks.onEventDecodeFailure).toHaveBeenCalledWith(
      "session_1",
      expect.objectContaining({ message: expect.stringContaining("remote_event_decode_failed") }),
    );
  });

  test("the global provider completion subject reaches the control-plane callback", async () => {
    const { client, callbacks } = recoveryClient();
    const testClient = client as unknown as {
      subscribeEvents(connection: unknown, generation: number): void;
    };
    async function* events() {
      yield {
        subject: "p.pair_1.evt._global",
        data: new TextEncoder().encode(
          JSON.stringify({
            type: "provider_config_changed",
            data: JSON.stringify({ revision: 9, providerId: "custom" }),
          }),
        ),
      };
      await new Promise(() => {});
    }
    testClient.subscribeEvents({ subscribe: () => events() }, 0);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(callbacks.onEvent).toHaveBeenCalledWith(
      expect.objectContaining({ type: "provider_config_changed" }),
      "_global",
    );
  });

  test.each(["subscribeTransfers", "subscribeLiveness", "subscribeState"] as const)(
    "%s reconnects when its iterator ends independently",
    async method => {
      const { client } = recoveryClient();
      const recovery = jest.fn();
      const testClient = client as unknown as {
        subscribeTransfers(connection: unknown, generation: number): void;
        subscribeLiveness(connection: unknown, generation: number): void;
        subscribeState(connection: unknown, generation: number): void;
        handleFailure(error: unknown): void;
      };
      testClient.handleFailure = recovery;
      async function* ended(): AsyncGenerator<never> {
        return;
      }

      testClient[method]({ subscribe: () => ended() }, 0);
      await new Promise(resolve => setTimeout(resolve, 0));

      expect(recovery).toHaveBeenCalledWith(expect.any(Error));
    },
  );

  test("several subscriptions ending in one generation trigger one reconnect", () => {
    const { client } = recoveryClient();
    const recovery = jest.fn();
    const testClient = client as unknown as {
      failGeneration(error: unknown, generation: number): void;
      handleFailure(error: unknown): void;
    };
    testClient.handleFailure = recovery;

    testClient.failGeneration(new Error("events ended"), 0);
    testClient.failGeneration(new Error("presence ended"), 0);

    expect(recovery).toHaveBeenCalledTimes(1);
  });
});

describe("RemoteClient request retry classification", () => {
  function response(data: unknown): Uint8Array {
    return new TextEncoder().encode(JSON.stringify(data));
  }

  test("retries transient NATS failures with one stable command id", async () => {
    jest.useFakeTimers();
    try {
      const { client } = recoveryClient();
      const request = jest
        .fn()
        .mockRejectedValueOnce(new NatsError("timeout", ErrorCode.Timeout))
        .mockResolvedValueOnce({ data: response({ success: true, data: { ok: true } }) });
      (client as unknown as { connection: { request: jest.Mock } | null }).connection = { request };

      const pending = client.requestRetry<{ ok: boolean }>({ type: "list_sessions" }, "list");
      await jest.runAllTimersAsync();
      await expect(pending).resolves.toMatchObject({ data: { ok: true } });
      expect(request).toHaveBeenCalledTimes(2);
      const first = JSON.parse(new TextDecoder().decode(request.mock.calls[0]?.[1]));
      const second = JSON.parse(new TextDecoder().decode(request.mock.calls[1]?.[1]));
      expect(first.id).toBe(second.id);
    } finally {
      jest.useRealTimers();
    }
  });

  test("does not retry a backend business error", async () => {
    const { client } = recoveryClient();
    const request = jest.fn().mockResolvedValue({
      data: response({ success: false, error: "session_is_running" }),
    });
    (client as unknown as { connection: { request: jest.Mock } | null }).connection = { request };

    await expect(client.requestRetry({ type: "abort" }, "list")).rejects.toThrow(
      "session_is_running",
    );
    expect(request).toHaveBeenCalledTimes(1);
  });
});

describe("RemoteClient OS lifecycle recovery", () => {
  test("foreground validates a healthy socket without replacing it", async () => {
    const { client } = recoveryClient();
    const flush = jest.fn().mockResolvedValue(undefined);
    const close = jest.fn().mockResolvedValue(undefined);
    const testClient = client as unknown as {
      connection: { flush(): Promise<void>; close(): Promise<void>; isClosed(): boolean } | null;
    };
    testClient.connection = { flush, close, isClosed: () => false };
    const open = jest.spyOn(client, "open").mockResolvedValue(undefined);

    await client.recoverNow("foreground");

    expect(flush).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });

  test("network path changes immediately replace the old generation", async () => {
    const { client } = recoveryClient();
    const close = jest.fn().mockResolvedValue(undefined);
    const testClient = client as unknown as {
      connection: { flush(): Promise<void>; close(): Promise<void>; isClosed(): boolean } | null;
    };
    testClient.connection = {
      flush: jest.fn().mockResolvedValue(undefined),
      close,
      isClosed: () => false,
    };
    const open = jest.spyOn(client, "open").mockResolvedValue(undefined);

    await client.recoverNow("network-changed");

    expect(close).toHaveBeenCalledTimes(1);
    expect(testClient.connection).toBeNull();
    expect(open).toHaveBeenCalledTimes(1);
  });

  test("offline pauses the socket and open attempts until reachability returns", async () => {
    const { client, callbacks } = recoveryClient();
    const close = jest.fn().mockResolvedValue(undefined);
    const testClient = client as unknown as {
      connection: { close(): Promise<void> } | null;
    };
    testClient.connection = { close };

    client.setNetworkAvailable(false);
    callbacks.onConnectionState.mockClear();
    await client.open();
    expect(close).toHaveBeenCalledTimes(1);
    expect(callbacks.onConnectionState).not.toHaveBeenCalled();

    const open = jest.spyOn(client, "open").mockResolvedValue(undefined);
    await client.recoverNow("network-restored");
    expect(open).toHaveBeenCalledTimes(1);
  });
});
