//! Generated model catalog.
//!
//! Models are stored as JSON and embedded with include_str!, so the compiler
//! only sees a &str — no 72K lines of struct literals to type-check.
//! Run `make generate-models` to update.

use serde::{Deserialize, Serialize};

/// Model mirrors the Go types.Model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api: String,
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub context_window: i32,
    pub max_tokens: i32,
    pub cost_input: f64,
    pub cost_output: f64,
    pub cost_cache_read: f64,
    pub cost_cache_write: f64,
    pub compat_json: String,
    pub tlm_json: String,
    pub headers_json: String,
    pub hide: bool,
}

/// INIT_BUILTIN_MODELS returns the complete built-in model catalog.
pub fn init_builtin_models() -> Vec<Model> {
    let data = include_str!("../models.json");
    serde_json::from_str(data).unwrap_or_default()
}
