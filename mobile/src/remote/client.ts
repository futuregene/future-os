import type { NatsConnection } from "nats.ws";
import { connect, jwtAuthenticator } from "nats.ws";
import { fromSeed } from "nkeys.js";
import { ensureFreshCredentials, refreshCredentials } from "./pairing";
import { jwtExpiry, randomId } from "./codec";
import {
  encodeBase64Url,
  handshakeTranscript,
  type HandshakeChallenge,
  verifyDesktopChallenge,
} from "./handshake";
import type { Presence, RemoteCommand, RemoteCredentials, RpcResponse, StreamEvent } from "./types";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const HANDSHAKE_PROTOCOL_VERSION = 1;

interface HandshakeConfirmation {
  confirmed: boolean;
  pairId: string;
  desktopId: string;
  bridgeInstanceId: string;
  deviceId: string;
  desktopNonce: string;
  presence: Presence;
}

export interface RemoteClientCallbacks {
  onCredentials(credentials: RemoteCredentials): void;
  onEvent(event: StreamEvent, sessionId: string): void;
  onPresence(presence: Presence): void;
  onConnectionState(state: "connected" | "reconnecting" | "disconnected"): void;
  onReconnected(): void;
  onError(error: Error): void;
}

function decodeJson<T>(data: Uint8Array): T {
  return JSON.parse(decoder.decode(data)) as T;
}

export class RemoteClient {
  private connection: NatsConnection | null = null;
  private credentials: RemoteCredentials;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private generation = 0;
  private stopped = false;
  private confirmedBridgeInstanceId = "";
  private handshakePromise: Promise<HandshakeConfirmation> | null = null;

  constructor(
    credentials: RemoteCredentials,
    private readonly callbacks: RemoteClientCallbacks,
  ) {
    this.credentials = credentials;
  }

  async open(): Promise<void> {
    this.stopped = false;
    this.credentials = await ensureFreshCredentials(this.credentials);
    await this.openSocket();
  }

  private async openSocket(): Promise<void> {
    const generation = ++this.generation;
    const seed = encoder.encode(this.credentials.seed);
    const connection = await connect({
      servers: this.credentials.natsWsUrl,
      inboxPrefix: `p.${this.credentials.pairId}.rep.${this.credentials.deviceId}`,
      authenticator: jwtAuthenticator(this.credentials.userJwt, seed),
    });
    if (this.stopped || generation !== this.generation) {
      await connection.close();
      return;
    }
    try {
      const confirmation = await this.ensureHandshake(connection);
      if (this.stopped || generation !== this.generation) {
        await connection.close();
        return;
      }
      this.connection = connection;
      this.callbacks.onCredentials(this.credentials);
      this.callbacks.onPresence(confirmation.presence);
      this.callbacks.onConnectionState("connected");
    } catch (error) {
      await connection.close();
      throw error;
    }
    this.subscribeEvents(connection, generation);
    this.subscribePresence(connection, generation);
    this.watchStatus(connection, generation);
    this.scheduleRefresh();
  }

  private subscribeEvents(connection: NatsConnection, generation: number): void {
    const subscription = connection.subscribe(`p.${this.credentials.pairId}.evt.>`);
    void (async () => {
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) break;
          const prefix = `p.${this.credentials.pairId}.evt.`;
          const sessionId = message.subject.startsWith(prefix)
            ? message.subject.slice(prefix.length)
            : "";
          this.callbacks.onEvent(decodeJson<StreamEvent>(message.data), sessionId);
        }
      } catch (error) {
        if (!this.stopped) this.callbacks.onError(asError(error));
      }
    })();
  }

  private subscribePresence(connection: NatsConnection, generation: number): void {
    const subscription = connection.subscribe(`p.${this.credentials.pairId}.presence`);
    void (async () => {
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) break;
          const presence = decodeJson<Presence>(message.data);
          if (
            !presence.bridgeInstanceId ||
            presence.bridgeInstanceId !== this.confirmedBridgeInstanceId
          ) {
            this.callbacks.onConnectionState("reconnecting");
            const confirmation = await this.ensureHandshake(connection);
            if (this.stopped || generation !== this.generation) break;
            this.callbacks.onPresence(confirmation.presence);
            this.callbacks.onConnectionState("connected");
            this.callbacks.onReconnected();
          } else {
            this.callbacks.onPresence(presence);
          }
        }
      } catch (error) {
        if (!this.stopped) this.callbacks.onError(asError(error));
      }
    })();
  }

  private watchStatus(connection: NatsConnection, generation: number): void {
    void (async () => {
      try {
        for await (const status of connection.status()) {
          if (this.stopped || generation !== this.generation) break;
          if (status.type === "disconnect") this.callbacks.onConnectionState("reconnecting");
          if (status.type === "reconnect") {
            try {
              const confirmation = await this.ensureHandshake(connection);
              if (this.stopped || generation !== this.generation) break;
              this.callbacks.onPresence(confirmation.presence);
              this.callbacks.onConnectionState("connected");
              this.callbacks.onReconnected();
            } catch (error) {
              this.callbacks.onError(asError(error));
            }
          }
        }
      } catch (error) {
        if (!this.stopped) this.callbacks.onError(asError(error));
      }
    })();
  }

  private scheduleRefresh(): void {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    const delay = Math.max(5_000, jwtExpiry(this.credentials.userJwt) * 1000 - Date.now() - 60_000);
    this.refreshTimer = setTimeout(() => {
      void this.refreshAndReconnect();
    }, delay);
  }

  private async refreshAndReconnect(): Promise<void> {
    if (this.stopped) return;
    try {
      this.callbacks.onConnectionState("reconnecting");
      const previous = this.connection;
      this.connection = null;
      this.credentials = await refreshCredentials(this.credentials);
      this.callbacks.onCredentials(this.credentials);
      if (previous) await previous.close();
      await this.openSocket();
      this.callbacks.onReconnected();
    } catch (error) {
      this.callbacks.onError(asError(error));
    }
  }

  async request<T>(
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
  ): Promise<RpcResponse<T>> {
    const connection = this.connection;
    if (!connection) throw new Error("not_connected");
    return this.requestWithConnection(connection, command, sessionId);
  }

  private async requestWithConnection<T>(
    connection: NatsConnection,
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
  ): Promise<RpcResponse<T>> {
    const payload = { ...command, id: command.id ?? randomId("cmd") };
    const message = await connection.request(
      `p.${this.credentials.pairId}.cmd.${sessionId || "new"}`,
      encoder.encode(JSON.stringify(payload)),
      { timeout: 10_000 },
    );
    const response = decodeJson<RpcResponse<T>>(message.data);
    if (!response.success) throw new Error(response.error ?? "command_failed");
    return response;
  }

  private async performHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    const keyPair = fromSeed(encoder.encode(this.credentials.seed));
    const clientPublicKey = keyPair.getPublicKey();
    const clientNonce = randomId("challenge");
    const challengeResponse = await this.requestWithConnection<HandshakeChallenge>(
      connection,
      {
        type: "pair_handshake",
        protocolVersion: HANDSHAKE_PROTOCOL_VERSION,
        pairId: this.credentials.pairId,
        deviceId: this.credentials.deviceId,
        clientPublicKey,
        clientNonce,
        expectedDesktopId: this.credentials.expectedDesktopId,
        expectedDesktopPublicKey: this.credentials.expectedDesktopPublicKey,
      },
      "handshake",
    );
    const challenge = challengeResponse.data;
    if (
      !verifyDesktopChallenge(challenge, {
        pairId: this.credentials.pairId,
        desktopId: this.credentials.expectedDesktopId,
        desktopPublicKey: this.credentials.expectedDesktopPublicKey,
        deviceId: this.credentials.deviceId,
        clientPublicKey,
        clientNonce,
      })
    ) {
      throw new Error("pairing_signature_invalid");
    }
    const transcript = handshakeTranscript(challenge);
    const clientSignature = encodeBase64Url(keyPair.sign(encoder.encode(transcript)));
    const confirmationResponse = await this.requestWithConnection<HandshakeConfirmation>(
      connection,
      {
        type: "pair_handshake_confirm",
        deviceId: this.credentials.deviceId,
        desktopNonce: challenge.desktopNonce,
        clientSignature,
      },
      "handshake",
    );
    const confirmation = confirmationResponse.data;
    if (
      !confirmation.confirmed ||
      confirmation.pairId !== this.credentials.pairId ||
      confirmation.desktopId !== this.credentials.expectedDesktopId ||
      confirmation.bridgeInstanceId !== challenge.bridgeInstanceId ||
      confirmation.deviceId !== this.credentials.deviceId ||
      confirmation.desktopNonce !== challenge.desktopNonce ||
      confirmation.presence.bridgeInstanceId !== challenge.bridgeInstanceId
    ) {
      throw new Error("pairing_confirmation_mismatch");
    }
    this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
    return confirmation;
  }

  private ensureHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    if (this.handshakePromise) return this.handshakePromise;
    const pending = this.performHandshake(connection);
    this.handshakePromise = pending;
    const clear = () => {
      if (this.handshakePromise === pending) this.handshakePromise = null;
    };
    void pending.then(clear, clear);
    return pending;
  }

  async close(): Promise<void> {
    this.stopped = true;
    this.generation += 1;
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = null;
    const connection = this.connection;
    this.connection = null;
    if (connection) await connection.close();
    this.callbacks.onConnectionState("disconnected");
  }
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}
