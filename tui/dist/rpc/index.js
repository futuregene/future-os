/**
 * RPC exports for xihu_tui Agent.
 * Only exports gRPC client (JSON-RPC and Unix socket removed).
 */
export * from "./types.js";
export { GrpcClient } from "./grpc-client.js";
