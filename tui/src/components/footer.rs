//! Footer — status bar matching the TS style. 1:1 port of
//! `tui/src/components/footer.ts`.
//!
//! Shows: pwd, model, thinking, token stats, cost, context usage.

#[cfg(test)]
use std::env;

use crate::theme::{Chrome, Theme};
use crate::tui::{Component, RESET};
use crate::utils::{truncate_to_width, visible_width, TruncateOptions};

#[derive(Debug, Clone, Default)]
pub struct FooterData {
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub streaming: bool,
    pub compacting: bool,
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

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Colorize with `fg` and reset to `base` afterwards (TS `colorFg`).
fn color_fg(c: u8, base: u8, text: &str) -> String {
    format!("\x1b[38;5;{c}m{text}\x1b[38;5;{base}m")
}

pub struct Footer {
    data: FooterData,
    theme: Theme,
    #[allow(dead_code)]
    width: usize,
}

impl Footer {
    pub fn new(width: usize) -> Self {
        Self {
            data: FooterData::default(),
            theme: Theme::default(),
            width,
        }
    }

    /// Adopt a palette (`/theme`). The status bar takes its colors from the
    /// [`Chrome`] view of it; the default palette is byte-identical to the
    /// ported TS renderer.
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = *theme;
    }

    /// The palette this footer paints with.
    pub fn theme(&self) -> Theme {
        self.theme
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
        let chrome = Chrome::from_theme(&self.theme);
        let base_fg = format!("\x1b[38;5;{}m", chrome.base);

        // Build left side: [spinner] [pwd] [model] [thinking]
        let mut left_parts: Vec<String> = Vec::new();

        // Spinner when streaming
        if self.data.streaming || self.data.compacting {
            let frame_idx = self.data.spinner_frame.unwrap_or(0) % SPINNER_FRAMES.len();
            left_parts.push(color_fg(
                chrome.accent,
                chrome.base,
                SPINNER_FRAMES[frame_idx],
            ));
        }

        if self.data.compacting {
            left_parts.push(color_fg(chrome.accent, chrome.base, "Compacting…"));
        }

        // Tool elapsed time
        if let Some(tool_elapsed) = self.data.tool_elapsed {
            if tool_elapsed > 0.0 {
                left_parts.push(color_fg(
                    chrome.token,
                    chrome.base,
                    &format!("{tool_elapsed}s"),
                ));
            }
        }

        // PWD — uses default fg (245)
        if let Some(cwd) = &self.data.cwd {
            let home = crate::home::home_dir();
            let pwd = home
                .as_deref()
                .and_then(|home| std::path::Path::new(cwd).strip_prefix(home).ok())
                .map(|rest| {
                    if rest.as_os_str().is_empty() {
                        "~".to_string()
                    } else {
                        format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
                    }
                })
                .unwrap_or_else(|| cwd.clone());
            left_parts.push(format!("{base_fg}{pwd}"));
        }

        // Model — brighter fg (252), optional thinking level in blue
        if let Some(model) = &self.data.model {
            let model_short = Self::shorten_model(model);
            let thinking = match self.data.thinking.as_deref() {
                Some(t) if !t.is_empty() && t != "off" => {
                    color_fg(chrome.thinking, chrome.base, &format!(" • {t}"))
                }
                _ => String::new(),
            };
            left_parts.push(color_fg(chrome.text, chrome.base, &model_short) + &thinking);
        }

        // Build right side: [token stats] [cost] [context usage]
        let mut right_parts: Vec<String> = Vec::new();

        // Token stats: Σ↑Xk ↓Xk — session *totals*. They have no ceiling
        // (every request resends the whole prompt), so they are marked with Σ
        // to keep them apart from the context readout below, which is the one
        // bounded level in this bar. The marker is glued to the first counter
        // and painted muted so it costs a single column: at 80 columns the
        // right side is already the part that gets truncated first, and a
        // longer marker would push the cost/context readouts out.
        let mut token_parts: Vec<String> = Vec::new();
        // JS truthiness: `if (this.data.tokensIn)` — a 0 value is falsy and
        // skipped. `if let Some` would render `↑0` for a Some(0).
        if let Some(n) = self.data.tokens_in {
            if n > 0 {
                token_parts.push(format!("↑{}", Self::fmt_tokens(n)));
            }
        }
        if let Some(n) = self.data.tokens_out {
            if n > 0 {
                token_parts.push(format!("↓{}", Self::fmt_tokens(n)));
            }
        }
        if let Some(n) = self.data.tokens_cache_r {
            if n > 0 {
                token_parts.push(format!("R{}", Self::fmt_tokens(n)));
            }
        }
        if let Some(n) = self.data.tokens_cache_w {
            if n > 0 {
                token_parts.push(format!("W{}", Self::fmt_tokens(n)));
            }
        }
        if !token_parts.is_empty() {
            let totals = color_fg(chrome.muted, chrome.base, "Σ")
                + &color_fg(chrome.token, chrome.base, &token_parts.join(" "));
            right_parts.push(totals);
        }

        // Cost
        if let Some(total_cost) = self.data.total_cost {
            if total_cost > 0.0 {
                right_parts.push(color_fg(
                    chrome.token,
                    chrome.base,
                    &format!("¥{total_cost:.3}"),
                ));
            }
        }

        // Context usage: tokenCount/contextWindow (color based on percent fill)
        // — the one bounded readout in the bar, left unmarked so that the Σ
        // totals above cannot be confused with it (see the token stats).
        if let Some(context_window) = self.data.context_window {
            // JS truthiness: 0 is falsy — skip zero windows.
            if context_window != 0 {
                let used = Self::fmt_tokens(self.data.context_tokens.unwrap_or(0));
                let win = Self::fmt_tokens(context_window);
                let pct = self.data.context_percent.unwrap_or(0);
                // Color based on usage level
                let used_color = if pct < 70 {
                    chrome.token // green < 70%
                } else if pct < 90 {
                    chrome.warn // yellow 70-90%
                } else {
                    chrome.error // red > 90%
                };
                let mut usage_str =
                    color_fg(used_color, chrome.base, &used) + &base_fg + &format!("/{win}");
                if self.data.auto_compaction_enabled {
                    usage_str += &color_fg(chrome.border, chrome.base, " (auto)");
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
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

    /// Restore HOME to a saved value (None = the variable was absent).
    fn restore_home(old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => env::set_var("HOME", v),
            None => env::remove_var("HOME"),
        }
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
    fn home_prefix_without_component_boundary_is_not_abbreviated() {
        let _guard = crate::test_env::lock();
        let home = crate::home::home_dir().unwrap();
        let cousin = std::path::PathBuf::from(format!("{}-other", home.display())).join("project");
        let line = render_footer(
            FooterData {
                cwd: Some(cousin.to_string_lossy().into_owned()),
                ..Default::default()
            },
            2000,
        );
        assert!(strip_ansi_codes(&line).contains(cousin.to_string_lossy().as_ref()));
    }

    #[test]
    fn renders_home_relative_cwd_with_tilde() {
        // Deterministic regardless of the ambient HOME: inject one under lock.
        // It must be absolute for the host (a POSIX path is not absolute on
        // Windows), and the abbreviation renders with the platform separator.
        let _guard = crate::test_env::lock();
        let old = env::var_os("HOME");
        let sep = std::path::MAIN_SEPARATOR;
        let home = if cfg!(windows) {
            "C:\\home\\tester".to_string()
        } else {
            "/home/tester".to_string()
        };
        env::set_var("HOME", &home);
        let line = render_footer(
            FooterData {
                cwd: Some(format!("{home}{sep}projects{sep}foo")),
                ..Default::default()
            },
            80,
        );
        restore_home(old);
        let text = strip_ansi_codes(&line);
        assert!(text.contains(&format!("~{sep}projects{sep}foo")), "{text}");
    }

    #[test]
    fn restore_home_handles_set_and_unset() {
        let _guard = crate::test_env::lock();
        let old = env::var_os("HOME");
        restore_home(Some(std::ffi::OsString::from("/tmp/home-probe")));
        assert_eq!(env::var("HOME").as_deref(), Ok("/tmp/home-probe"));
        restore_home(None);
        assert!(env::var_os("HOME").is_none());
        restore_home(old);
    }

    #[test]
    fn renders_raw_cwd_when_home_unset() {
        let _guard = crate::test_env::lock();
        let old = env::var_os("HOME");
        env::remove_var("HOME");
        let line = render_footer(
            FooterData {
                cwd: Some("/some/where".into()),
                ..Default::default()
            },
            80,
        );
        restore_home(old);
        assert!(strip_ansi_codes(&line).contains("/some/where"));
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
    fn renders_token_stats_as_marked_session_totals() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(5000),
                tokens_out: Some(12000),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("Σ↑5k ↓12k"), "{text}");
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
        assert!(text.contains("50k/128k"), "{text}");
        // With no session totals to mark, no Σ is rendered either.
        assert!(!text.contains('Σ'), "{text}");
    }

    /// The two token readouts answer different questions — the Σ counters are
    /// session totals that have no ceiling, the unmarked `used/window` readout
    /// is the bounded occupancy — so only the totals carry a marker, and the
    /// whole right-hand side is pinned as one string.
    #[test]
    fn the_footer_marks_the_session_totals_apart_from_the_context_readout() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(5000),
                tokens_out: Some(12000),
                tokens_cache_r: Some(3000),
                tokens_cache_w: Some(2000),
                total_cost: Some(0.5),
                context_tokens: Some(50000),
                context_window: Some(128000),
                context_percent: Some(39),
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(
            text.contains("Σ↑5k ↓12k R3k W2k  ¥0.500  50k/128k"),
            "{text}"
        );
        // Exactly one Σ, and it opens the totals group — the context readout
        // stays unmarked, so the two cannot be read as one dimension.
        assert_eq!(text.matches('Σ').count(), 1, "{text}");
        let totals = text.find('Σ').expect("the totals are marked");
        let context = text.find("50k/128k").expect("the context readout is there");
        assert!(totals < context, "{text}");
        assert!(
            !text[context..].contains('Σ'),
            "the context readout carries no totals marker: {text}"
        );
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
    fn renders_compaction_without_a_running_turn() {
        let line = render_footer(
            FooterData {
                compacting: true,
                ..Default::default()
            },
            80,
        );
        let text = strip_ansi_codes(&line);
        assert!(text.contains("Compacting…"));
        assert!(text.contains("⠋"));
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

    /// At the 80-column default the right side is the first thing truncated, so
    /// the Σ marker costs exactly one column, and on the byte-pinned rows above
    /// it is paid out of the gap (there is a 59-column gap left at 120 columns).
    /// This is the heavy case — four counters, a cost, an `(auto)` marker and a
    /// deep cwd — where the truncation is already active: the cost and the
    /// context usage still show, and the one column comes off the window figure.
    #[test]
    fn the_marker_costs_one_column_and_keeps_cost_and_context_visible() {
        let line = render_footer(
            FooterData {
                cwd: Some("/Users/geilige/future-os/.worktrees/tui-parity".into()),
                model: Some("deepseek-v4-flash".into()),
                thinking: Some("high".into()),
                tokens_in: Some(812_000),
                tokens_out: Some(45_000),
                tokens_cache_r: Some(700_000),
                tokens_cache_w: Some(12_000),
                total_cost: Some(12.345),
                context_tokens: Some(48_000),
                context_window: Some(200_000),
                context_percent: Some(24),
                auto_compaction_enabled: true,
                ..Default::default()
            },
            80,
        );
        assert!(visible_width(&line) <= 80, "{line:?}");
        let text = strip_ansi_codes(&line);
        assert!(text.contains("Σ↑812k ↓45k R700k W12k"), "{text}");
        assert!(text.contains("¥12.345"), "the cost survives: {text}");
        assert!(text.contains("48k/"), "the context usage survives: {text}");
        // A marker cannot be free: Σ costs exactly one column, and on this
        // saturated row that column comes off the *tail* — the cost in the
        // middle and the used-token figure are untouched, the model name on the
        // left is already clipped by the same rule. Before the marker the row
        // ended `...  48k/200` (the half-width cap), so one window digit goes.
        // The full-width rows above pin the rest of the right side byte for byte.
        assert!(text.ends_with("48k/20"), "{text}");
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
        assert!(text.contains("50k/128k"), "{text}");
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
        assert!(text.contains("ΣR3k W2k"), "{text}");
    }

    /// JS truthiness: `if (this.data.tokensCacheR)` skips a zero value, so a
    /// `Some(0)` must NOT render `R0`/`W0` (or `↑0`/`↓0`). The P4 tmux
    /// harness caught the port rendering `R0 W0` where TS renders nothing.
    #[test]
    fn zero_token_stats_are_skipped_js_truthiness() {
        let line = render_footer(
            FooterData {
                tokens_in: Some(0),
                tokens_out: Some(0),
                tokens_cache_r: Some(0),
                tokens_cache_w: Some(0),
                ..Default::default()
            },
            120,
        );
        let text = strip_ansi_codes(&line);
        assert!(!text.contains("R0"));
        assert!(!text.contains("W0"));
        assert!(!text.contains("↑0"));
        assert!(!text.contains("↓0"));
        // All-zero totals render nothing at all — not even the Σ marker.
        assert!(!text.contains('Σ'), "{text}");
    }

    #[test]
    fn set_width_and_height_accessors() {
        let mut footer = Footer::new(80);
        footer.set_width(120);
        assert_eq!(footer.get_height(), 1);
        footer.invalidate(); // no cache — a documented no-op
    }

    #[test]
    fn context_usage_color_follows_percent_thresholds() {
        for (pct, color) in [(50usize, 71u8), (75, 226), (95, 204)] {
            let line = render_footer(
                FooterData {
                    context_tokens: Some(100),
                    context_window: Some(1000),
                    context_percent: Some(pct),
                    ..Default::default()
                },
                120,
            );
            assert!(
                line.contains(&format!("\x1b[38;5;{color}m")),
                "pct {pct} should use color {color}: {line:?}"
            );
        }
    }

    #[test]
    fn auto_compaction_appends_marker() {
        let line = render_footer(
            FooterData {
                context_tokens: Some(100),
                context_window: Some(1000),
                context_percent: Some(10),
                auto_compaction_enabled: true,
                ..Default::default()
            },
            120,
        );
        assert!(strip_ansi_codes(&line).contains("(auto)"));
    }

    #[test]
    fn as_any_downcasts_to_footer() {
        let mut footer = Footer::new(80);
        assert!(footer.as_any().downcast_ref::<Footer>().is_some());
        assert!(footer.as_any_mut().downcast_mut::<Footer>().is_some());
    }

    /// A busy footer: every branch of `render` that paints something.
    fn busy_footer_data() -> FooterData {
        FooterData {
            model: Some("openai/gpt-4o".into()),
            thinking: Some("high".into()),
            streaming: true,
            compacting: true,
            spinner_frame: Some(0),
            tool_elapsed: Some(2.0),
            tokens_in: Some(5000),
            total_cost: Some(1.5),
            context_tokens: Some(95),
            context_window: Some(100),
            context_percent: Some(95),
            auto_compaction_enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn theme_round_trips_and_the_default_palette_stays_byte_identical() {
        let data = busy_footer_data();
        let default_line = render_footer(data.clone(), 120);

        // Literal bytes, not a round-trip: `Footer::new` already carries
        // `Theme::default()`, so re-applying the default palette to it would be
        // true by construction and could never fail. These are the exact SGR
        // bytes the ported TS footer emits for a busy row — legacy accent 39
        // (spinner, "Compacting…"), base 245 (repeated because `colorFg`
        // resets to it and the `join` separator emits it again), token green 71
        // ("2s", "↑5k", "¥1.500"), bright text 252, thinking blue 117, the
        // muted 241 "Σ" that marks the token counters as session totals rather
        // than a level, error red 204 at a 95 % context fill and the muted 240
        // "(auto)" marker.
        let expected = format!(
            "\x1b[38;5;39m⠋\x1b[38;5;245m\x1b[38;5;245m  \
             \x1b[38;5;39mCompacting…\x1b[38;5;245m\x1b[38;5;245m  \
             \x1b[38;5;71m2s\x1b[38;5;245m\x1b[38;5;245m  \
             \x1b[38;5;252mgpt-4o\x1b[38;5;245m\x1b[38;5;117m • high\x1b[38;5;245m\
             \x1b[38;5;245m{}\
             \x1b[38;5;241mΣ\x1b[38;5;245m\x1b[38;5;71m↑5k\x1b[38;5;245m\x1b[38;5;245m  \
             \x1b[38;5;71m¥1.500\x1b[38;5;245m\x1b[38;5;245m  \
             \x1b[38;5;204m95\x1b[38;5;245m\x1b[38;5;245m/100\x1b[38;5;240m (auto)\
             \x1b[38;5;245m\x1b[m",
            " ".repeat(59)
        );
        assert_eq!(default_line, expected);

        let mut footer = Footer::new(120);
        footer.set_data(data);
        footer.set_theme(&crate::theme::DARK_THEME);
        assert_eq!(footer.theme(), crate::theme::DARK_THEME);
        assert_eq!(footer.render(120).remove(0), default_line);

        let light = crate::themes::theme_by_id("light").expect("light is in the catalog");
        footer.set_theme(&light);
        assert_eq!(footer.theme(), light);
        let themed = footer.render(120).remove(0);
        assert_ne!(themed, default_line, "a light palette must recolor the bar");
        // The spinner comes from the applied accent, not the legacy table.
        assert!(
            themed.contains(&format!("\x1b[38;5;{}m", light.accent)),
            "{themed:?}"
        );
        assert!(!themed.contains("\x1b[38;5;39m"), "{themed:?}");
    }

    #[test]
    fn themed_footer_paints_every_role_from_the_palette() {
        let theme = crate::theme::Theme {
            dim: 200,
            fg: 201,
            accent: 202,
            thinking_medium: 203,
            success: 204,
            thinking_high: 205,
            error: 206,
            border: 207,
            ..crate::theme::DARK_THEME
        };
        let mut footer = Footer::new(120);
        footer.set_theme(&theme);

        // 95 % context → base 200, text 201, accent 202 (spinner + compacting),
        // thinking 203, token 204 (tokens + cost), error 206, border 207 (auto).
        footer.set_data(busy_footer_data());
        let line = footer.render(120).remove(0);
        for role in [200u8, 201, 202, 203, 204, 206, 207] {
            assert!(
                line.contains(&format!("\x1b[38;5;{role}m")),
                "role {role} missing: {line:?}"
            );
        }

        // 75 % context → the warning yellow (205).
        footer.set_data(FooterData {
            context_tokens: Some(75),
            context_window: Some(100),
            context_percent: Some(75),
            ..Default::default()
        });
        let line = footer.render(120).remove(0);
        assert!(line.contains("\x1b[38;5;205m"), "{line:?}");
    }

    #[test]
    fn cwd_equal_to_home_renders_as_tilde() {
        let _guard = crate::test_env::lock();
        let old = env::var_os("HOME");
        let home = crate::home::home_dir().unwrap();
        let line = render_footer(
            FooterData {
                cwd: Some(home.to_string_lossy().into_owned()),
                ..Default::default()
            },
            80,
        );
        restore_home(old);
        assert!(strip_ansi_codes(&line).contains('~'), "{line:?}");
    }
}
