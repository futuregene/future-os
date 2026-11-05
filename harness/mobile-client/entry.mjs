/**
 * The phone's own remote client, running outside a phone.
 *
 * The mobile app reaches a desktop through `src/remote/pairing.ts` (claim an
 * invitation, refresh the NATS credential) and `src/remote/client.ts` (the
 * transport, the catalogue, the command channel). Both are plain TypeScript over
 * an HTTP client and a WebSocket, so they run in Node as they do on the device —
 * what is missing is the platform layer, and that is what the shims below
 * replace.
 *
 * This is deliberately the real code, not a re-implementation: a bug in the
 * phone's pairing (an accepted-but-wrong `ws://` endpoint, a credential that is
 * not refreshed, a catalogue that never applies) has to be able to fail here.
 * The only substituted pieces are `expo-device`, `expo-secure-store` and
 * `react-native`, each of which answers the app with a constant that a test can
 * state: this device's name, an in-memory key store, and "we are on iOS".
 *
 * Protocol: one command per line on stdin, one JSON result per line on stdout.
 * A long-lived process, because a connection, its presence beats and a second
 * client taking over are all things that only exist over time.
 */
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { claimPairingCode } from "../../mobile/src/remote/pairing.ts";
import { RemoteClient } from "../../mobile/src/remote/client.ts";

const state = {
  credentials: null,
  client: null,
  sessions: [],
  workspaces: [],
  presence: null,
  features: [],
  connection: null,
  errors: [],
  events: [],
};

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

/** Wait for a predicate over `state`, with a deadline rather than a sleep. */
async function until(predicate, timeoutMs, what) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate())
      return true;
    await new Promise(done => setTimeout(done, 100));
  }
  throw new Error(`timed out waiting for ${what}: ${JSON.stringify({
    connection: state.connection,
    sessions: state.sessions.length,
    presence: state.presence,
    errors: state.errors.slice(-2),
  })}`);
}

function startClient() {
  const client = new RemoteClient(state.credentials, {
    onCredentials: async (fresh) => {
      state.credentials = fresh;
    },
    onEvent: (event, sessionId) => {
      // Kept short: a case asserts on the few things a phone shows (a reply
      // arriving, a run finishing), not on the whole projection.
      state.events.push({ kind: event.kind ?? event.type ?? "event", sessionId });
      if (state.events.length > 200)
        state.events.shift();
    },
    onEventDecodeFailure: (_sessionId, error) => state.errors.push(String(error?.message ?? error)),
    onPresence: presence => {
      state.presence = presence;
    },
    onSessions: sessions => {
      state.sessions = sessions;
    },
    // This client declares `catalog_delta_v1`, so the host answers with rows that
    // moved rather than the whole catalogue — and a wrapper that only implements
    // `onSessions` receives nothing at all. Merging by id is what the app's own
    // reducer does; a full packet replaces, `removed` deletes.
    onSessionDelta: (packet) => {
      const byId = new Map(packet.full ? [] : state.sessions.map(row => [row.sessionId, row]));
      for (const row of packet.rows ?? [])
        byId.set(row.sessionId, row);
      for (const id of packet.removed ?? [])
        byId.delete(id);
      state.sessions = [...byId.values()];
    },
    onSessionActivity: (activity) => {
      state.activity = activity;
    },
    onWorkspaces: workspaces => {
      state.workspaces = workspaces;
    },
    onFeatures: features => {
      state.features = features;
    },
    onConnectionState: (connection) => {
      state.connection = connection;
    },
    onReconnected: () => {
      state.connection = { phase: "ready", reconnected: true };
    },
    onError: error => state.errors.push(String(error?.message ?? error)),
  });
  state.client = client;
  return client;
}

const commands = {
  async claim({ invitation }) {
    // The exact call the phone makes after a QR scan: parse the link, generate
    // the device identity, claim it, and keep the credentials.
    state.credentials = await claimPairingCode(invitation, AbortSignal.timeout(20_000));
    return {
      pairId: state.credentials.pairId,
      desktopId: state.credentials.expectedDesktopId,
      natsWsUrl: state.credentials.natsWsUrl,
      tokenUrl: state.credentials.tokenUrl,
    };
  },

  async connect() {
    if (!state.client)
      startClient();
    await state.client.open();
    return { ok: true };
  },

  async ready({ timeoutMs = 30_000 }) {
    // The catalogue arrives two ways: pushed on the delta subject, or answered by
    // an explicit `list_sessions`. The app issues that request itself when it
    // wants the list now (see `useSessionCatalog`), so the harness does the same
    // rather than waiting for a push that may only come on the next change.
    if (!state.sessions.length) {
      try {
        const response = await state.client.request({ type: "list_sessions" }, "list", 15_000);
        const rows = response?.data?.sessions ?? response?.sessions ?? [];
        if (Array.isArray(rows) && rows.length)
          state.sessions = rows;
        else
          state.errors.push(`list_sessions returned no rows: ${JSON.stringify(response).slice(0, 600)}`);
      }
      catch (error) {
        state.errors.push(`list_sessions: ${String(error?.message ?? error)}`);
      }
    }
    await until(() => state.sessions.length > 0, timeoutMs, "a session catalogue");
    return {
      sessions: state.sessions.length,
      workspaces: state.workspaces.length,
      features: state.features,
      connection: state.connection,
    };
  },

  snapshot() {
    return {
      credentialPairId: state.credentials?.pairId ?? null,
      deviceId: state.credentials?.deviceId ?? null,
      sessions: state.sessions.map(row => ({
        sessionId: row.sessionId,
        threadId: row.threadId ?? null,
        title: row.title ?? null,
        status: row.status ?? null,
        workspaceId: row.workspaceId ?? null,
        pinned: row.pinned ?? null,
        updatedAt: row.updatedAt ?? null,
      })),
      workspaces: state.workspaces.map(row => ({ id: row.id ?? null, name: row.name ?? null })),
      presence: state.presence,
      features: state.features,
      connection: state.connection,
      errors: state.errors.slice(-5),
      events: state.events.slice(-20),
    };
  },

  visible({ sessionId }) {
    state.client.setVisibleSession(sessionId);
    return { ok: true };
  },

  /** The phone's command channel: the same call `useSessionCatalog` makes. */
  async request({ command, sessionId }) {
    const response = await state.client.request(command, sessionId ?? "list", 15_000);
    return { ok: response?.ok !== false, response };
  },

  async disconnect() {
    await state.client?.close("UserInitiated");
    return { ok: true };
  },

  events() {
    return { events: state.events };
  },
};

const input = createInterface({ input: process.stdin });
const ready = () => send({ hello: true, pid: process.pid });

input.on("line", async (line) => {
  let message;
  try {
    message = JSON.parse(line);
  }
  catch {
    send({ id: null, ok: false, error: "unparsable command" });
    return;
  }
  const handler = commands[message.cmd];
  if (!handler) {
    send({ id: message.id, ok: false, error: `unknown command ${message.cmd}` });
    return;
  }
  try {
    const result = await handler(message);
    send({ id: message.id, ok: true, result });
  }
  catch (error) {
    send({ id: message.id, ok: false, error: String(error?.message ?? error) });
  }
});

// A case may need the device identity to survive a restart; the harness keeps
// that on disk, so accept it before the first command.
if (process.env.HARNESS_DEVICE_SEED_FILE) {
  try {
    readFileSync(process.env.HARNESS_DEVICE_SEED_FILE, "utf8");
  }
  catch {}
}
ready();
