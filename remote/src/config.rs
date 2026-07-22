//! Bridge configuration (L0: read from environment variables, with defaults).

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

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var tests must run serially (they mutate shared process env).
    // Combining into one test avoids the parallel-thread race.
    #[test]
    fn config_defaults_and_env_override() {
        let orig_nats = std::env::var("FUTURE_NATS_URL").ok();
        let orig_agent = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        let orig_pair = std::env::var("FUTURE_REMOTE_PAIR_ID").ok();

        // ── defaults (vars cleared) ──
        std::env::remove_var("FUTURE_NATS_URL");
        std::env::remove_var("FUTURE_AGENT_GRPC_ADDR");
        std::env::remove_var("FUTURE_REMOTE_PAIR_ID");
        let c = Config::from_env();
        assert_eq!(c.nats_url, "nats://localhost:4222");
        assert_eq!(c.agent_addr, "127.0.0.1:50051");
        assert_eq!(c.pair_id, "DEVPAIR");

        // ── env override ──
        std::env::set_var("FUTURE_NATS_URL", "nats://custom:4222");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "10.0.0.1:9999");
        std::env::set_var("FUTURE_REMOTE_PAIR_ID", "TESTPAIR");
        let c = Config::from_env();
        assert_eq!(c.nats_url, "nats://custom:4222");
        assert_eq!(c.agent_addr, "10.0.0.1:9999");
        assert_eq!(c.pair_id, "TESTPAIR");

        // Restore
        if let Some(v) = orig_nats { std::env::set_var("FUTURE_NATS_URL", v); } else { std::env::remove_var("FUTURE_NATS_URL"); }
        if let Some(v) = orig_agent { std::env::set_var("FUTURE_AGENT_GRPC_ADDR", v); } else { std::env::remove_var("FUTURE_AGENT_GRPC_ADDR"); }
        if let Some(v) = orig_pair { std::env::set_var("FUTURE_REMOTE_PAIR_ID", v); } else { std::env::remove_var("FUTURE_REMOTE_PAIR_ID"); }
    }
}
