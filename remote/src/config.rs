//! Bridge 配置（L0：从环境变量读，带默认值）。

pub struct Config {
    pub nats_url: String,
    pub agent_addr: String,
    pub pair_id: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            nats_url: std::env::var("FUTURE_NATS_URL")
                .unwrap_or_else(|_| "nats://localhost:4222".into()),
            agent_addr: std::env::var("FUTURE_AGENT_GRPC_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:50051".into()),
            pair_id: std::env::var("FUTURE_REMOTE_PAIR_ID").unwrap_or_else(|_| "DEVPAIR".into()),
        }
    }
}
