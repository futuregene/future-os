//! xihu-agent — Rust implementation of the xihu agent backend

pub mod agent;
pub mod compaction;
pub mod config;
pub mod engine;
pub mod events;
pub mod llm;
pub mod prompt;
pub mod rpc;
pub mod session;
pub mod skills;
pub mod tools;
pub mod types;
pub mod utils;

pub use agent::Loop;
pub use config::{merge_settings, load_settings, Settings};
pub use engine::{Engine, EngineConfig};
pub use events::EventBus;
pub use llm::Client as LLMClient;
pub use rpc::Server;
pub use session::{Manager, Session, SessionEntry};
pub use skills::{discover_skills, Skill};
pub use tools::all_tools;
pub use types::{AgentMessage, AgentTool, LLMProvider, Message, StreamEvent, ToolDef};
pub use utils::{default_config_dir, default_session_dir, generate_id};
