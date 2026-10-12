import type { MutableRefObject } from "react";
import type { RemoteClient } from "./client";
import type { RemoteCredentials } from "./types";

interface ConversationRefs {
  conversationEpochRef: MutableRefObject<number>;
  clientRef: MutableRefObject<RemoteClient | null>;
  credentialsRef: MutableRefObject<RemoteCredentials | null>;
}

/** Credential renewal keeps a conversation; navigation and client replacement end it. */
export function captureConversationLifetime({ conversationEpochRef, clientRef, credentialsRef }: ConversationRefs) {
  const epoch = conversationEpochRef.current;
  const client = clientRef.current;
  const pairId = credentialsRef.current?.pairId;
  const expectedDesktopId = credentialsRef.current?.expectedDesktopId;
  return () => conversationEpochRef.current === epoch
    && clientRef.current === client
    && credentialsRef.current?.pairId === pairId
    && credentialsRef.current?.expectedDesktopId === expectedDesktopId;
}
