//! future-agent — Rust implementation of the FutureAgent agent backend

pub mod agent;
pub mod auth;
pub mod compaction;
pub mod config;
pub mod engine;
pub mod events;
pub mod grpc;
pub mod llm;
pub mod models;
pub mod prompt;
pub mod rpc;
pub mod sandbox;
pub mod session;
pub mod skills;
pub mod tools;
pub mod types;
pub mod utils;

pub use agent::Loop;
pub use auth::AuthStore;
pub use config::{load_settings, Settings};
pub use engine::{Engine, EngineConfig};
pub use events::EventBus;
pub use llm::Client as LLMClient;
pub use models::{get_default_model, Registry as ModelRegistry};
pub use rpc::ServerSession;
pub use session::{Manager, Session, SessionEntry};
pub use skills::{discover_skills, Skill, AGENTS_SKILLS_DIR, APP_SKILLS_DIR, PROJECT_SKILLS_DIR};
pub use tools::{all_tools, coding_tools};
pub use types::{AgentMessage, AgentTool, LLMProvider, Message, StreamEvent, ToolDef};
pub use utils::{default_config_dir, default_session_dir, generate_id};
