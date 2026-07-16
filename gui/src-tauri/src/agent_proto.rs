// Generated gRPC bindings — checked into src/generated/.
// Not every proto message is constructed by the GUI (e.g. server-only types),
// so silence dead-code warnings for the whole module.
#![allow(dead_code)]

mod proto {
    include!("generated/proto.rs");
}

pub use proto::future_agent_client::FutureAgentClient;
pub use proto::*;
