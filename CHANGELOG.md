# Changelog

All notable changes to xihu will be documented in this file.

## [0.1.0] - 2026-05-09

### Added
- Initial release of xihu, a Go-based AI coding assistant
- Full TUI with Bubble Tea: chat viewport, editor, footer, overlays
- Agent loop with streaming support and parallel tool execution
- 30+ CLI flags aligned with pi-mono
- Session management with JSONL persistence
- Model selector with type-to-filter and provider info
- Scoped models management with reordering
- Session tree view with fold/unfold and filter modes
- Session selector with rename, delete, sort, and path display
- Settings panel with live toggles and sub-menus
- Thinking level cycling (off/minimal/low/medium/high/xhigh)
- External editor support (Ctrl+G)
- Clipboard paste (Ctrl+V) with large paste markers
- Direct bash execution (! and !! commands)
- Slash command autocomplete
- Markdown rendering via glamour
- Diff rendering with intra-line word-level highlighting
- Tool call and tool result rendering with expand/collapse
- Scroll indicators in editor and overlay
- Help overlay (Ctrl+H) with scrollable keybinding reference
- Theme system with dark and light themes
- Configurable keybindings via KeyMap
