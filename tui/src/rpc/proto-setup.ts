// Inject Long globally for protobufjs before any gRPC modules load.
// protobufjs does global lookup (dcodeIO.Long) instead of require("long"),
// which fails in bun build --compile because the global isn't set.
// This file must be imported before @grpc/grpc-js or @grpc/proto-loader.
import Long from "long";

(globalThis as Record<string, unknown>).Long = Long;
(globalThis as Record<string, unknown>).dcodeIO = { Long };
