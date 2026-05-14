export * from "./types.js";

// Export RpcClient and GrpcClient explicitly
export { RpcClient } from "./client.js";
export { GrpcClient } from "./grpc-client.js";

export type { EventListener } from "./client.js";

// Factory function for creating clients
import { RpcClient } from "./client.js";
import { GrpcClient } from "./grpc-client.js";

export function createClient(type: "http" | "grpc" = "grpc", addr?: string): RpcClient | GrpcClient {
  if (type === "grpc") {
    const grpcAddr = addr ?? process.env.XIHU_GRPC_ADDR ?? "localhost:50051";
    console.log(`Using gRPC client connecting to ${grpcAddr}`);
    return new GrpcClient(grpcAddr);
  }
  console.log("Using HTTP client");
  return new RpcClient();
}
