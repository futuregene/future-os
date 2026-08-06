//! Footer — status bar matching the TS style. 1:1 port of
//! `tui/src/components/footer.ts`.
//!
//! Shows: pwd, model, thinking, token stats, cost, context usage.

use std::env;

use crate::tui::{Component, RESET};
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

#[derive(Debug, Clone, Default)]
pub struct FooterData {
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub streaming: bool,
    pub spinner_frame: Option<usize>,
    pub pending: Option<usize>,
    pub context_tokens: Option<usize>,
    pub context_window: Option<usize>,
    pub context_percent: Option<usize>,
    pub tokens_in: Option<usize>,
    pub tokens_out: Option<usize>,
    pub tokens_cache_r: Option<usize>,
    pub tokens_cache_w: Option<usize>,
    pub tool_elapsed: Option<f64>,
    pub total_cost: Option<f64>,
    pub auto_compaction_enabled: bool,
}

const BASE_FG: u8 = 245;
const ACCENT_FG: u8 = 252;
const THINKING_FG: u8 = 117;
const TOKEN_FG: u8 = 71;
const COST_FG: u8 = 71;
const GREEN_FG: u8 = 71;
const YELLOW_FG: u8 = 226;
const RED_FG: u8 = 204;
const AUTO_FG: u8 = 240;
const SPINNER_FG: u8 = 39;

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Colorize with fg and reset to BASE_FG afterwards (TS `colorFg`).
fn color_fg(c: u8, text: &str) -> String {
    format!("\x1b[38;5;{c}m{text}\x1b[38;5;{BASE_FG}m")
}

pub struct Footer {
    data: FooterData,
    #[allow(dead_code)]
    width: usize,
}

impl Footer {
    pub fn new(width: usize) -> Self {
        Self {
            data: FooterData::default(),
            width,
        }
    }

    pub fn set_data(&mut self, data: FooterData) {
        self.data = data;
    }

    pub fn set_width(&mut self, _w: usize) {}

    pub fn get_height(&self) -> usize {
        1
    }

    fn shorten_model(model: &str) -> String {
        model.rsplit('/').next().unwrap_or(model).to_string()
    }

    fn fmt_tokens(n: usize) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{}k", (n as f64 / 1_000.0).round() as u64)
        } else {
            n.to_string()
        }
    }
}

impl Component for Footer {
    fn render(&mut self, width: usize) -> Vec<String> {
        let base_fg = format!("\x1b[38;5;{BASE_FG}m");

        // Build left side: [spinner] [pwd] [model] [thinking]
        let mut left_parts: Vec<String> = Vec::new();

        // Spinner when streaming
        if self.data.streaming {
            let frame_idx = self.data.spinner_frame.unwrap_or(0) % SPINNER_FRAMES.len();
            left_parts.push(color_fg(SPINNER_FG, SPINNER_FRAMES[frame_idx]));
        }

        // Tool elapsed time
        if let Some(tool_elapsed) = self.data.tool_elapsed {
            if tool_elapsed > 0.0 {
                left_parts.push(color_fg(TOKEN_FG, &format!("{tool_elapsed}s")));
            }
        }

        // PWD — uses default fg (245)
        if let Some(cwd) = &self.data.cwd {
            let home = env::var("HOME").unwrap_or_default();
            let pwd = if !home.is_empty() && cwd.starts_with(&home) {
                format!("~{}", &cwd[home.len()..])
            } else {
                cwd.clone()
            };
            left_parts.push(format!("{base_fg}{pwd}"));
        }

        // Model — brighter fg (252), optional thinking level in blue
        if let Some(model) = &self.data.model {
            let model_short = Self::shorten_model(model);
            let thinking = match self.data.thinking.as_deref() {
                Some(t) if !t.is_empty() && t != "off" => color_fg(THINKING_FG, &format!(" • {t}")),
                _ => String::new(),
            };
            left_parts.push(color_fg(ACCENT_FG, &model_short) + &thinking);
        }

        // Build right side: [token stats] [cost] [context usage]
        let mut right_parts: Vec<String> = Vec::new();

        // Token stats: ↑Xk ↓Xk
        let mut token_parts: Vec<String> = Vec::new();
        if let Some(n) = self.data.tokens_in {
            token_parts.push(format!("↑{}", Self::fmt_tokens(n)));
        }
        if let Some(n) = self.data.tokens_out {
            token_parts.push(format!("↓{}", Self::fmt_tokens(n)));
        }
        if let Some(n) = self.data.tokens_cache_r {
            token_parts.push(format!("R{}", Self::fmt_tokens(n)));
        }
        if let Some(n) = self.data.tokens_cache_w {
            token_parts.push(format!("W{}", Self::fmt_tokens(n)));
        }
        if !token_parts.is_empty() {
            right_parts.push(color_fg(TOKEN_FG, &token_parts.join(" ")));
        }

        // Cost
        if let Some(total_cost) = self.data.total_cost {
            if total_cost > 0.0 {
                right_parts.push(color_fg(COST_FG, &format!("¥{total_cost:.3}")));
            }
        }

        // Context usage: tokenCount/contextWindow (color based on percent fill)
        if let Some(context_window) = self.data.context_window {
            // JS truthiness: 0 is falsy — skip zero windows.
            if context_window != 0 {
                let used = Self::fmt_tokens(self.data.context_tokens.unwrap_or(0));
                let win = Self::fmt_tokens(context_window);
                let pct = self.data.context_percent.unwrap_or(0);
                // Color based on usage level
                let used_color = if pct < 70 {
                    GREEN_FG // green < 70%
                } else if pct < 90 {
                    YELLOW_FG // yellow 70-90%
                } else {
                    RED_FG // red > 90%
                };
                let mut usage_str = color_fg(used_color, &used) + &base_fg + &format!("/{win}");
                if self.data.auto_compaction_enabled {
                    usage_str += &color_fg(AUTO_FG, " (auto)");
                }
                right_parts.push(usage_str);
            }
        }

        let left = left_parts.join(&format!("{base_fg}  "));
        let right = right_parts.join(&format!("{base_fg}  "));

        // Ensure the left part starts with baseFg even if leftParts is empty
        let mut left_str = if left_parts.is_empty() {
            base_fg.clone()
        } else {
            left
        };

        let mut left_len = visible_width(&left_str);
        let right_len = visible_width(&right);
        let avail = width.saturating_sub(1); // reserve 1 for safety margin

        // Both sides must be truncated on overflow: an over-wide line wraps
        // physically and desyncs the diff renderer's row tracking, which
        // assumes one logical line == one terminal row. Share the space —
        // the right side (tokens/cost/context) gets at most half so it stays
        // visible even with a deep cwd or long model name.
        let right_str = if left_len + right_len > avail {
            let max_right = right_len.min(avail / 2);
            truncate_to_width(&right, max_right, &TruncateOptions::default())
        } else {
            right
        };
        if left_len + right_len > avail {
            let max_left = (avail as i64 - visible_width(&right_str) as i64 - 1).max(0) as usize;
            // No ellipsis: the styled left string may end with an ANSI
            // sequence, and truncateToWidth's ellipsis replaces the last byte
            // — which could be the tail of an escape sequence and corrupt it.
            left_str = truncate_to_width(&left_str, max_left, &TruncateOptions::default());
            left_len = visible_width(&left_str);
        }

        let padding =
            (width as i64 - left_len as i64 - visible_width(&right_str) as i64 - 1).max(1) as usize;
        let line = format!("{left_str}{base_fg}{}{right_str}", " ".repeat(padding));

        vec![format!("{line}{RESET}")]
    }

    fn invalidate(&mut self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::strip_ansi_codes;

    fn render_footer(data: FooterData, width: usize) -> String {
        let mut footer = Footer::new(width);
        footer.set_data(data);
        footer.render(width).remove(0)
    }

    #[test]
    fn renders_with_minimal_data() {
        let line = render_footer(FooterData::default(), 80);
        assert!(visible_width(&line) <= 80);
    }

    #[test]
    fn renders_cwd() {
        let line = render_footer(
            FooterData {
                cwd: Some("/home/user/project".into()),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("/home/user/project"));
    }

    #[test]
    fn renders_home_relative_cwd_with_tilde() {
        // Deterministic regardless of the ambient HOME: inject one under lock.
        let _guard = crate::test_env::ENV_LOCK.lock();
        let old = env::var_os("HOME");
        env::set_var("HOME", "/home/tester");
        let line = render_footer(
            FooterData {
                cwd: Some("/home/tester/projects/foo".into()),
                ..Default::default()
            },
            80,
        );
        if let Some(old) = old {
            env::set_var("HOME", old);
        } else {
            env::remove_var("HOME");
        }
        let text = strip_ansi_codes(&line);
        assert!(text.contains("~/projects/foo"));
    }

    #[test]
    fn renders_model_name_shortened() {
        let line = render_footer(
            FooterData {
                model: Some("anthropic/claude-sonnet-4".into()),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("claude-sonnet-4"));
        assert!(!text.contains("anthropic/"));
    }

    #[test]
    fn renders_thinking_level_when_not_off() {
        let line = render_footer(
            FooterData {
                model: Some("openai/gpt-4o".into()),
                thinking: Some("high".into()),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("high"));
    }

    #[test]
    fn does_not_render_thinking_level_when_off() {
        let line = render_footer(
            FooterData {
                model: Some("openai/gpt-4o".into()),
                thinking: Some("off".into()),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(!text.contains("off"));
    }

    #[test]
    fn renders_token_stats() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(5000),
                tokens_out: Some(12000),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("↑5k"));
        assert!(text.contains("↓12k"));
    }

    #[test]
    fn renders_cost() {
        let line = render_footer(
            FooterData {
                total_cost: Some(0.1234),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("¥0.123"));
    }

    #[test]
    fn renders_context_usage() {
        let line = render_footer(
            FooterData {
                context_tokens: Some(50000),
                context_window: Some(128000),
                context_percent: Some(39),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("50k"));
        assert!(text.contains("128k"));
    }

    #[test]
    fn renders_auto_compaction_indicator() {
        let line = render_footer(
            FooterData {
                context_tokens: Some(50000),
                context_window: Some(128000),
                context_percent: Some(39),
                auto_compaction_enabled: true,
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("(auto)"));
    }

    #[test]
    fn renders_spinner_when_streaming() {
        let line = render_footer(
            FooterData {
                streaming: true,
                spinner_frame: Some(0),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("⠋"));
    }

    #[test]
    fn renders_tool_elapsed_time() {
        let line = render_footer(
            FooterData {
                tool_elapsed: Some(5.0),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("5s"));
    }

    #[test]
    fn never_exceeds_terminal_width() {
        let line = render_footer(
            FooterData {
                cwd: Some(
                    "/very/long/path/to/a/deeply/nested/directory/structure/that/keeps/going"
                        .into(),
                ),
                model: Some("anthropic/claude-sonnet-4-20250514".into()),
                thinking: Some("xhigh".into()),
                streaming: true,
                spinner_frame: Some(0),
                tokens_in: Some(999000),
                tokens_out: Some(999000),
                total_cost: Some(12.345),
                context_tokens: Some(99000),
                context_window: Some(128000),
                context_percent: Some(77),
                auto_compaction_enabled: true,
                ..Default::default()
            },
            40,
        );
        assert!(visible_width(&line) <= 40);
    }

    #[test]
    fn right_side_stays_visible_with_long_cwd() {
        let line = render_footer(
            FooterData {
                cwd: Some(
                    "/extremely/deeply/nested/path/that/goes/on/and/on/forever/and/ever".into(),
                ),
                model: Some("openai/gpt-4o".into()),
                context_tokens: Some(50000),
                context_window: Some(128000),
                context_percent: Some(39),
                ..Default::default()
            },
            60,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("50k"));
        assert!(text.contains("128k"));
    }

    #[test]
    fn fmt_tokens_formats_large_numbers() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(1_500_000),
                tokens_out: Some(500),
                ..Default::default()
            },
            120,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("1.5M"));
        assert!(text.contains("500"));
    }

    #[test]
    fn fmt_tokens_formats_small_numbers() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(42),
                tokens_out: Some(999),
                ..Default::default()
            },
            120,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("42"));
        assert!(text.contains("999"));
    }

    #[test]
    fn get_height_is_always_1() {
        let footer = Footer::new(80);
        assert_eq!(footer.get_height(), 1);
    }

    #[test]
    fn cache_token_stats_render() {
        let line = render_footer(
            FooterData {
                tokens_cache_r: Some(3000),
                tokens_cache_w: Some(2000),
                ..Default::default()
            },
            120,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("R3k"));
        assert!(text.contains("W2k"));
    }
}
