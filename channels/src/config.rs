//! Unified channel configuration.
//! Reads from ~/.future/channels/config.json

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChannelConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub feishu: Option<FeishuChannelConfig>,
    #[serde(default)]
    pub dingtalk: Option<DingtalkChannelConfig>,
    /// Configuration for every other channel, keyed by channel id.
    ///
    /// One block per channel keeps this file open-ended: a new channel is a new
    /// key, not a new field in a schema every version has to agree on. Each
    /// channel deserializes its own block and reports a precise error if it does
    /// not fit.
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, serde_json::Value>,
}

impl ChannelConfig {
    /// The config block for one channel.
    ///
    /// A channel with a legacy top-level block (`feishu`, `dingtalk`) gets that
    /// unless an explicit `providers.<id>` block exists, which wins — so users
    /// can migrate one channel at a time without a flag day.
    pub fn provider_config(&self, id: &str) -> Option<serde_json::Value> {
        if let Some(block) = self.providers.get(id) {
            return Some(block.clone());
        }
        match id {
            "feishu" => self
                .feishu
                .as_ref()
                .and_then(|block| serde_json::to_value(block).ok()),
            "dingtalk" => self
                .dingtalk
                .as_ref()
                .and_then(|block| serde_json::to_value(block).ok()),
            _ => None,
        }
    }

    /// Whether a channel's block asks to be started.
    pub fn provider_enabled(block: &serde_json::Value) -> bool {
        block
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// Read configuration for a read-only command.
    ///
    /// Unlike [`Self::load`] this has no first-run side effect and never fails:
    /// `future channel list` and `status` must work on a machine that has never
    /// started the bridge, and a broken file must not stop them from reporting
    /// what they can. The note explains anything the caller should tell the user.
    pub fn load_for_read() -> (Self, Option<String>) {
        let path = Self::default_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(config) => (config, None),
                Err(error) => (
                    Self::default(),
                    Some(format!(
                        "cannot parse {}: {error}; reporting defaults",
                        path.display()
                    )),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                Self::default(),
                Some(format!(
                    "no configuration at {}; every channel is unconfigured",
                    path.display()
                )),
            ),
            Err(error) => (
                Self::default(),
                Some(format!(
                    "cannot read {}: {error}; reporting defaults",
                    path.display()
                )),
            ),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,
    #[serde(default = "default_cwd")]
    pub cwd: String,
    /// Default model for channel sessions (e.g. "deepseek-flash").
    /// If empty, the agent's boot-time default is used.
    #[serde(default = "default_model")]
    pub model: String,
    /// Default thinking level: "off", "minimal", "low", "medium", "high", "xhigh".
    #[serde(default = "default_thinking_level")]
    pub thinking_level: String,
    /// Default permission level: "all", "workspace", "none".
    #[serde(default = "default_permission_level")]
    pub permission_level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeishuChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    #[serde(default)]
    pub dm_allowlist: Vec<String>,
    #[serde(default = "default_group_policy")]
    pub group_policy: String,
    #[serde(default)]
    pub group_allowlist: Vec<String>,
    #[serde(default = "default_true")]
    pub require_mention: bool,
    #[serde(default = "default_true")]
    pub streaming: bool,
    #[serde(default = "default_true")]
    pub resolve_sender_names: bool,
    #[serde(default = "default_max_image_mb")]
    pub max_image_mb: u64,
    #[serde(default)]
    pub typing_indicator: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DingtalkChannelConfig {
    /// Sender IDs permitted to use the agent in DMs or groups. Empty denies all.
    /// Use ["*"] only when every sender with access to the bot is trusted.
    #[serde(default)]
    pub sender_allowlist: Vec<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_dingtalk_domain")]
    pub domain: String,
}

fn default_dingtalk_domain() -> String {
    "api.dingtalk.com".into()
}

impl Default for DingtalkChannelConfig {
    fn default() -> Self {
        Self {
            sender_allowlist: Vec::new(),
            enabled: false,
            client_id: String::new(),
            client_secret: String::new(),
            domain: default_dingtalk_domain(),
        }
    }
}

// ─── Defaults ──────────────────────────────────────────────────────────────

fn default_grpc_addr() -> String {
    future_rpc::transport::AUTO_ENDPOINT.into()
}
fn default_cwd() -> String {
    home_dir().to_string_lossy().into_owned()
}
fn default_model() -> String {
    "future/deepseek-v4-pro".into()
}
fn default_thinking_level() -> String {
    "xhigh".into()
}
fn default_permission_level() -> String {
    "all".into()
}
fn default_domain() -> String {
    "feishu".into()
}
fn default_dm_policy() -> String {
    "allowlist".into()
}
fn default_group_policy() -> String {
    "disabled".into()
}
fn default_true() -> bool {
    true
}
fn default_max_image_mb() -> u64 {
    10
}

// ─── Load / Save ───────────────────────────────────────────────────────────

impl ChannelConfig {
    pub fn default_path() -> PathBuf {
        home_dir()
            .join(".future")
            .join("channels")
            .join("config.json")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::default_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let config: Self = serde_json::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                // (map-chain, not if-let: rustfmt explodes single-line
                // if-lets and the false-edge brace is unreachable — the
                // config path always has a parent directory.)
                path.parent().map(std::fs::create_dir_all).transpose()?;
                std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
                anyhow::bail!(
                    "Default config written to {}. Edit it and restart.",
                    path.display()
                );
            }
            Err(e) => Err(anyhow::anyhow!(
                "Failed to read config at {}: {}",
                path.display(),
                e
            )),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            grpc_addr: default_grpc_addr(),
            cwd: default_cwd(),
            model: default_model(),
            thinking_level: default_thinking_level(),
            permission_level: default_permission_level(),
        }
    }
}

impl Default for FeishuChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: String::new(),
            app_secret: String::new(),
            domain: default_domain(),
            dm_policy: default_dm_policy(),
            dm_allowlist: vec![],
            group_policy: default_group_policy(),
            group_allowlist: vec![],
            require_mention: true,
            streaming: true,
            resolve_sender_names: true,
            max_image_mb: 10,
            typing_indicator: false,
        }
    }
}

/// Home directory for channel data and config.
///
/// The environment wins over the platform profile: `$HOME` (POSIX, and a
/// redirected portable home), then `USERPROFILE` (Windows shells set no
/// `HOME`), then `dirs::home_dir()`. Windows `dirs` reads the token profile,
/// which observes neither variable — a `$HOME`-only lookup fell back to the
/// real profile (or a literal `~` directory next to the cwd, silently writing
/// the config there) and an isolated test home never took effect.
pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .into_iter()
        .chain(std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty() && path.is_absolute())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("~"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── AgentConfig defaults ────────────────────────────────────────────────

    #[test]
    fn agent_config_defaults() {
        let c = AgentConfig::default();
        assert_eq!(c.grpc_addr, "auto");
        assert_eq!(c.model, "future/deepseek-v4-pro");
        assert_eq!(c.thinking_level, "xhigh");
        assert_eq!(c.permission_level, "all");
        assert!(!c.cwd.is_empty());
    }

    // ─── FeishuChannelConfig defaults ────────────────────────────────────────

    #[test]
    fn feishu_config_defaults() {
        let c = FeishuChannelConfig::default();
        assert!(!c.enabled);
        assert!(c.app_id.is_empty());
        assert!(c.app_secret.is_empty());
        assert_eq!(c.domain, "feishu");
        assert_eq!(c.dm_policy, "allowlist");
        assert_eq!(c.group_policy, "disabled");
        assert!(c.require_mention);
        assert!(c.streaming);
        assert!(c.resolve_sender_names);
        assert_eq!(c.max_image_mb, 10);
        assert!(!c.typing_indicator);
    }

    // ─── DingtalkChannelConfig defaults ──────────────────────────────────────

    #[test]
    fn dingtalk_config_defaults() {
        let c = DingtalkChannelConfig::default();
        assert!(!c.enabled);
        assert!(c.client_id.is_empty());
        assert!(c.client_secret.is_empty());
        assert_eq!(c.domain, "api.dingtalk.com");
    }

    // ─── ChannelConfig defaults ──────────────────────────────────────────────

    #[test]
    fn channel_config_default() {
        let c = ChannelConfig::default();
        assert_eq!(c.agent.grpc_addr, "auto");
        assert!(c.feishu.is_none());
        assert!(c.dingtalk.is_none());
    }

    // ─── JSON deserialization ────────────────────────────────────────────────

    #[test]
    fn deserialize_empty_json() {
        let c: ChannelConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.agent.grpc_addr, "auto");
        assert!(c.feishu.is_none());
        assert!(c.dingtalk.is_none());
    }

    #[test]
    fn deserialize_full_config() {
        let json = r#"{
            "agent": {
                "grpc_addr": "http://localhost:50051",
                "cwd": "/home/user",
                "model": "openai/gpt-4o",
                "thinking_level": "high",
                "permission_level": "workspace"
            },
            "feishu": {
                "enabled": true,
                "app_id": "cli_test",
                "app_secret": "secret_test",
                "domain": "feishu",
                "dm_policy": "open",
                "dm_allowlist": ["user1"],
                "group_policy": "open",
                "group_allowlist": ["chat1"],
                "require_mention": false,
                "streaming": false,
                "resolve_sender_names": false,
                "max_image_mb": 5,
                "typing_indicator": true
            },
            "dingtalk": {
                "enabled": true,
                "client_id": "ding_id",
                "client_secret": "ding_secret",
                "domain": "custom.dingtalk.com"
            }
        }"#;
        let c: ChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.agent.model, "openai/gpt-4o");
        assert_eq!(c.agent.thinking_level, "high");
        let feishu = c.feishu.unwrap();
        assert!(feishu.enabled);
        assert_eq!(feishu.app_id, "cli_test");
        assert!(!feishu.require_mention);
        assert!(!feishu.streaming);
        assert_eq!(feishu.max_image_mb, 5);
        assert!(feishu.typing_indicator);
        let dingtalk = c.dingtalk.unwrap();
        assert!(dingtalk.enabled);
        assert_eq!(dingtalk.domain, "custom.dingtalk.com");
    }

    #[test]
    fn deserialize_partial_feishu() {
        let json = r#"{"feishu": {"enabled": true, "app_id": "test"}}"#;
        let c: ChannelConfig = serde_json::from_str(json).unwrap();
        let feishu = c.feishu.unwrap();
        assert!(feishu.enabled);
        assert_eq!(feishu.app_id, "test");
        assert!(feishu.app_secret.is_empty()); // default
        assert!(feishu.streaming); // default true
    }

    // ─── Provider blocks ─────────────────────────────────────────────────────

    #[test]
    fn a_provider_block_is_read_from_the_providers_map() {
        let mut config = ChannelConfig::default();
        config.providers.insert(
            "telegram".to_string(),
            serde_json::json!({"enabled": true, "bot_token": "x"}),
        );
        let block = config.provider_config("telegram").expect("block");
        assert_eq!(block["bot_token"], "x");
        assert!(ChannelConfig::provider_enabled(&block));
        assert!(config.provider_config("missing").is_none());
    }

    #[test]
    fn the_legacy_top_level_blocks_still_resolve() {
        // Feishu and DingTalk predate the `providers` map; both shapes must work
        // so users can migrate one channel at a time.
        let mut config = ChannelConfig::default();
        config.feishu = Some(FeishuChannelConfig {
            enabled: true,
            app_id: "app".into(),
            ..Default::default()
        });
        config.dingtalk = Some(DingtalkChannelConfig {
            enabled: true,
            client_id: "id".into(),
            ..Default::default()
        });
        assert!(ChannelConfig::provider_enabled(
            &config.provider_config("feishu").expect("feishu")
        ));
        assert!(ChannelConfig::provider_enabled(
            &config.provider_config("dingtalk").expect("dingtalk")
        ));
        // A channel with neither shape configured has no block at all.
        assert!(config.provider_config("cli").is_none());
    }

    #[test]
    fn an_explicit_provider_block_wins_over_the_legacy_one() {
        let mut config = ChannelConfig::default();
        config.feishu = Some(FeishuChannelConfig {
            enabled: false,
            app_id: "legacy".into(),
            ..Default::default()
        });
        config.providers.insert(
            "feishu".to_string(),
            serde_json::json!({"enabled": true, "app_id": "new"}),
        );
        let block = config.provider_config("feishu").expect("block");
        assert_eq!(block["app_id"], "new");
        assert!(ChannelConfig::provider_enabled(&block));
    }

    #[test]
    fn an_enabled_flag_requires_a_boolean() {
        assert!(!ChannelConfig::provider_enabled(&serde_json::json!({})));
        assert!(!ChannelConfig::provider_enabled(&serde_json::json!({
            "enabled": "yes"
        })));
        assert!(ChannelConfig::provider_enabled(&serde_json::json!({
            "enabled": true
        })));
    }

    // ─── load_for_read ───────────────────────────────────────────────────────

    #[test]
    fn reading_a_missing_config_yields_defaults_with_a_note() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("config-read-missing");
        let (config, note) = ChannelConfig::load_for_read();
        let note = note.expect("a missing file must be explained");
        assert!(note.contains("no configuration"), "{note}");
        assert!(config.providers.is_empty());
        // Crucially: no side effect. The template is only written by `load`.
        assert!(!home.path.join(".future/channels/config.json").exists());
    }

    #[test]
    fn reading_a_broken_config_reports_it_instead_of_failing() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("config-read-broken");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{not json").unwrap();
        let (config, note) = ChannelConfig::load_for_read();
        assert!(config.providers.is_empty());
        let note = note.expect("a note");
        assert!(note.contains("cannot parse"), "{note}");
    }

    #[test]
    fn reading_an_unreadable_config_reports_it_instead_of_failing() {
        // A directory where the file should be: reading fails with an error that
        // is neither "missing" nor "malformed", and must still be reported.
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("config-read-unreadable");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(dir.join("config.json")).unwrap();
        let (config, note) = ChannelConfig::load_for_read();
        assert!(config.providers.is_empty());
        let note = note.expect("a note");
        assert!(note.contains("cannot read"), "{note}");
    }

    #[test]
    fn reading_a_valid_config_returns_it_without_a_note() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("config-read-ok");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"providers": {"cli": {"enabled": true}}}"#,
        )
        .unwrap();
        let (config, note) = ChannelConfig::load_for_read();
        assert!(note.is_none(), "{note:?}");
        assert!(ChannelConfig::provider_enabled(
            &config.provider_config("cli").expect("cli")
        ));
    }

    // ─── Roundtrip ───────────────────────────────────────────────────────────

    #[test]
    fn config_roundtrip() {
        let original = ChannelConfig {
            agent: AgentConfig {
                grpc_addr: "http://test:9999".into(),
                cwd: "/tmp".into(),
                model: "test/model".into(),
                thinking_level: "low".into(),
                permission_level: "none".into(),
            },
            feishu: Some(FeishuChannelConfig {
                enabled: true,
                app_id: "app1".into(),
                app_secret: "sec1".into(),
                ..Default::default()
            }),
            dingtalk: None,
            providers: std::collections::BTreeMap::new(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: ChannelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent.grpc_addr, "http://test:9999");
        assert_eq!(restored.agent.model, "test/model");
        assert!(restored.feishu.as_ref().unwrap().enabled);
    }

    // ─── default_path ────────────────────────────────────────────────────────

    #[test]
    fn default_path_contains_channels() {
        let path = ChannelConfig::default_path();
        assert!(path.to_string_lossy().contains(".future"));
        assert!(path.to_string_lossy().contains("channels"));
        assert!(path.to_string_lossy().ends_with("config.json"));
    }

    // ─── load ────────────────────────────────────────────────────────────────

    #[test]
    fn load_writes_defaults_when_missing() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("cfg-missing");
        let err = ChannelConfig::load().unwrap_err();
        assert!(
            err.to_string().contains("Default config written"),
            "unexpected error: {err}"
        );
        // The default file was actually written to the isolated home.
        let path = home
            .path
            .join(".future")
            .join("channels")
            .join("config.json");
        assert!(path.exists(), "default config must be written");
        let written = std::fs::read_to_string(path).unwrap();
        let parsed: ChannelConfig = serde_json::from_str(&written).unwrap();
        assert!(parsed.feishu.is_none());
    }

    #[test]
    fn load_parses_existing_file() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("cfg-valid");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"agent":{"model":"x/y","grpc_addr":"http://test:1"}}"#,
        )
        .unwrap();
        let cfg = ChannelConfig::load().unwrap();
        assert_eq!(cfg.agent.model, "x/y");
        assert_eq!(cfg.agent.grpc_addr, "http://test:1");
    }

    #[test]
    fn load_rejects_invalid_json() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("cfg-invalid");
        let dir = home.path.join(".future").join("channels");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "not json").unwrap();
        let err = ChannelConfig::load().unwrap_err();
        assert!(
            err.to_string().contains("Failed to parse"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_read_error_other_than_not_found() {
        let _guard = crate::test_support::home_lock();
        let home = crate::test_support::IsolatedHome::new("cfg-readerr");
        let dir = home.path.join(".future").join("channels");
        // A directory at the config path makes read_to_string fail with an
        // error kind other than NotFound.
        std::fs::create_dir_all(dir.join("config.json")).unwrap();
        let err = ChannelConfig::load().unwrap_err();
        assert!(
            err.to_string().contains("Failed to read config"),
            "unexpected error: {err}"
        );
    }

    // ─── home_dir fallback ───────────────────────────────────────────────────

    #[test]
    fn home_dir_returns_nonempty() {
        let p = home_dir();
        assert!(!p.as_os_str().is_empty());
    }
}
