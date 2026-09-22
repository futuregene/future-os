//! RPC layer — port of `tui/src/rpc/`.
//!
//! P1: typed `ModelInfo` surface for the components layer.
//! P3: full types + tonic gRPC client (`GrpcClient`) — 1:1 port of
//! `rpc/grpc-client.ts` (persistent event stream, reconnect, heartbeat,
//! deadline-bounded unary calls).

pub mod grpc_client;
pub mod provider_types;
pub mod types;

pub use grpc_client::{grpc_addr, GrpcClient};
pub use provider_types::{
    parse_providers_response, validate_provider_id, validate_provider_input, ProviderInfo,
    ProviderInput, ProviderModelInput,
};
pub use types::*;
