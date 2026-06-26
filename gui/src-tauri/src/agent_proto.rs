// Generated gRPC bindings — not every proto message is constructed by the GUI
// (e.g. server-only types), so silence dead-code warnings for the whole module.
#![allow(dead_code)]

tonic::include_proto!("proto");

// Re-export for compatibility with the rest of the GUI codebase.
pub use future_agent_client::FutureAgentClient;
