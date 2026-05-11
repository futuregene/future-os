// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/utils"
)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showWelcome(msg WelcomeMsg) {
	m.setTerminalTitle()
	if m.quietStartup {
		return
	}

	// Store welcome data for rebuild on toggle
	m.lastWelcomeMsg = &msg

	accentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(msg.ThemeAccent)).
		Bold(true)
	dimStyle := lipgloss.NewStyle().
		Faint(true)

	// ── Logo + instructions (TS pi-mono: builtInHeader ExpandableText) ──────
	// Logo: "xihu vX.X.X" — bold accent + dim (TS: theme.bold(theme.fg("accent", APP_NAME)) + theme.fg("dim", ` v${version}`))
	// Prepend a leading spacer entry so the viewport doesn't clip the first line.
	m.chat.AppendSystem("")
	m.chat.AppendSystem(accentStyle.Render("xihu") + dimStyle.Render(" v"+utils.Version))

	if m.welcomeExpanded {
		m.renderExpandedInstructions()
	} else {
		m.renderCompactInstructions()
	}

	// ── Onboarding text (TS pi-mono: "Pi can explain its own features...") ──
	m.chat.AppendSystem("")
	m.chat.AppendSystem(dimStyle.Render("Xihu can explain its own features and look up its docs. Ask it how to use or extend Xihu."))

	// ── Changelog (async) ──────────────────────────────────────────────────
	m.checkChangelog()

	// ── Version / tmux checks (async) ──────────────────────────────────────
	go m.checkNewVersion()
	go m.checkTmuxKeyboard()

	// ── Settings/model errors (TS pi-mono: models.json / settings errors) ──
	if msg.SettingsError != "" {
		m.chat.AppendError("settings error: " + msg.SettingsError)
	}

	// ── Loaded resources (TS pi-mono: showLoadedResources) ─────────────────
	m.showLoadedResources()

	// ── Diagnostics ────────────────────────────────────────────────────────
	if len(msg.SkillCollisions) > 0 || len(msg.ExtensionDiagnostics) > 0 || len(msg.KeybindingConflicts) > 0 {
		m.showLoadedDiagnostics(msg.SkillCollisions, msg.ExtensionDiagnostics, msg.KeybindingConflicts)
	}
}

// rebuildWelcome clears the welcome entries and re-renders them.
// Called when welcomeExpanded is toggled (Ctrl+H).
func (m *AppModel) rebuildWelcome() {
	if m.lastWelcomeMsg == nil {
		return
	}
	msg := *m.lastWelcomeMsg

	// Clear previous welcome entries from chat.
	// We remove all system-type entries up to (but not including) diagnostics.
	// A simpler approach: clear all system entries and re-add.
	entries := m.chat.GetEntries()
	// Find the range of welcome entries: from the first system entry
	// (logo "xihu v...") up to the last resource/diagnostic entry.
	// We rebuild all non-streaming, non-user entries.
	var keep []int // indices to keep
	inWelcome := false
	afterWelcome := false
	for i, e := range entries {
		if !afterWelcome {
			if e.Type == "system" || e.Type == "error" || e.Type == "warning" {
				inWelcome = true
				continue // skip welcome entries
			}
			if inWelcome && (e.Type == "custom_message" || e.Type == "user_message") {
				afterWelcome = true
			}
		}
		if afterWelcome || !inWelcome {
			keep = append(keep, i)
		}
	}
	if len(keep) > 0 {
		m.chat.KeepEntries(keep)
	} else {
		m.chat.Clear()
	}

	// Re-render
	m.showWelcome(msg)
}

// renderCompactInstructions renders the collapsed-mode keybinding hints.
// TS pi-mono compactInstructions format:
//
//	escape interrupt · ctrl+c/ctrl+d clear/exit · / commands · ! bash · ctrl+o more
func (m *AppModel) renderCompactInstructions() {
	dimStyle := lipgloss.NewStyle().Faint(true)

	interruptKey := formatKeyStr(m.keybindings, GlobalInterrupt)
	if interruptKey == "" {
		interruptKey = "escape"
	}
	clearKey := formatKeyStr(m.keybindings, GlobalClear)
	if clearKey == "" {
		clearKey = "ctrl+c"
	}
	exitKey := formatKeyStr(m.keybindings, GlobalExit)
	if exitKey == "" {
		exitKey = "ctrl+d"
	}
	moreKey := formatKeyStr(m.keybindings, GlobalToggleTools)
	if moreKey == "" {
		moreKey = "ctrl+o"
	}

	// Normalize to lowercase for TS-style display
	interruptKey = strings.ToLower(interruptKey)
	clearKey = strings.ToLower(clearKey)
	exitKey = strings.ToLower(exitKey)
	moreKey = strings.ToLower(moreKey)

	compactInstructions := fmt.Sprintf("%s interrupt · %s/%s clear/exit · / commands · ! bash · %s more",
		interruptKey, clearKey, exitKey, moreKey)
	m.chat.AppendSystem(dimStyle.Render(compactInstructions))

	// Expand hint (TS: "Press ctrl+o to show full startup help and loaded resources.")
	m.chat.AppendSystem(dimStyle.Render("Press " + moreKey + " to show full startup help and loaded resources."))
}

// renderExpandedInstructions renders the full keybinding list.
// TS pi-mono expandedInstructions: 16 keybinding lines.
func (m *AppModel) renderExpandedInstructions() {
	dimStyle := lipgloss.NewStyle().Faint(true)

	hint := func(binding KeybindingID, description string) string {
		key := formatKeyStr(m.keybindings, binding)
		if key == "" {
			key = string(defaultKeyForBinding(binding))
		}
		return dimStyle.Render(strings.ToLower(key) + " to " + description)
	}
	rawHint := func(key string, description string) string {
		return dimStyle.Render(key + " to " + description)
	}

	// Match TS pi-mono expandedInstructions order exactly
	instructions := []string{
		hint(GlobalInterrupt, "interrupt"),
		hint(GlobalClear, "clear"),
		rawHint(strings.ToLower(formatKeyStrDefault(m.keybindings, GlobalClear, "ctrl+c"))+" twice", "to exit"),
		hint(GlobalExit, "to exit (empty)"),
		rawHint("ctrl+z", "to suspend"),
		rawHint(strings.ToLower(formatKeyStrDefault(m.keybindings, EditorDeleteToLineEnd, "ctrl+k")), "to delete to end"),
		rawHint(strings.ToLower(formatKeyStrDefault(m.keybindings, GlobalCycleThinking, "shift+tab")), "to cycle thinking level"),
		rawHint(
			strings.ToLower(formatKeyStrDefault(m.keybindings, GlobalCycleModelFwd, "ctrl+p"))+"/"+
				strings.ToLower(formatKeyStrDefault(m.keybindings, GlobalCycleModelBack, "shift+ctrl+p")),
			"to cycle models",
		),
		hint(GlobalModelSelector, "to select model"),
		hint(GlobalToggleTools, "to expand tools"),
		hint(GlobalToggleThinking, "to expand thinking"),
		hint(GlobalExternalEditor, "for external editor"),
		rawHint("/", "for commands"),
		rawHint("!", "to run bash"),
		rawHint("!!", "to run bash (no context)"),
		rawHint("alt+enter", "to queue follow-up"),
		rawHint("alt+up", "to edit all queued messages"),
		rawHint("ctrl+v", "to paste image"),
		rawHint("drop files", "to attach"),
	}

	for _, line := range instructions {
		m.chat.AppendSystem(line)
	}
}

// formatKeyStrDefault is like formatKeyStr but returns a default if not found.
func formatKeyStrDefault(kb *KeybindingsManager, binding KeybindingID, defaultKey string) string {
	key := formatKeyStr(kb, binding)
	if key == "" {
		return defaultKey
	}
	return key
}

// defaultKeyForBinding returns the default key string for a binding ID.
func defaultKeyForBinding(binding KeybindingID) string {
	switch binding {
	case GlobalInterrupt:
		return "escape"
	case GlobalClear:
		return "ctrl+c"
	case GlobalExit:
		return "ctrl+d"
	case GlobalToggleTools:
		return "ctrl+o"
	case GlobalToggleThinking:
		return "ctrl+t"
	case GlobalExternalEditor:
		return "ctrl+g"
	case GlobalModelSelector:
		return "ctrl+l"
	case GlobalCycleModelFwd:
		return "ctrl+p"
	case GlobalCycleModelBack:
		return "shift+ctrl+p"
	case GlobalCycleThinking:
		return "shift+tab"
	case GlobalToggleHeader:
		return "ctrl+h"
	case EditorDeleteToLineEnd:
		return "ctrl+k"
	default:
		return "?"
	}
}

// showLoadedResources renders the [Skills], [Extensions], [Context], [Prompts],
// [Themes] sections in collapsed compact format.
// Always shown (not just in expanded mode), matching TS pi-mono showLoadedResources.
func (m *AppModel) showLoadedResources() {
	if m.lastWelcomeMsg == nil {
		return
	}
	msg := *m.lastWelcomeMsg

	dimStyle := lipgloss.NewStyle().Faint(true)
	sectionHeader := func(name string) string {
		return lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.MdHeading)).Render("[" + name + "]")
	}
	compactList := func(items []string) string {
		// Filter empty, sort
		filtered := make([]string, 0, len(items))
		for _, item := range items {
			item = strings.TrimSpace(item)
			if item != "" {
				filtered = append(filtered, item)
			}
		}
		return dimStyle.Render("  " + strings.Join(filtered, ", "))
	}

	// ── [Context] section (TS: shown first) ──────────────────────────────
	if len(msg.ContextFiles) > 0 {
		contextCompact := make([]string, len(msg.ContextFiles))
		for i, fp := range msg.ContextFiles {
			contextCompact[i] = formatContextPath(fp)
		}
		m.chat.AppendSystem("")
		m.chat.AppendSystem(sectionHeader("Context"))
		m.chat.AppendSystem(compactList(contextCompact))
	}

	// ── [Skills] section ─────────────────────────────────────────────────
	if len(msg.Skills) > 0 {
		skillNames := make([]string, len(msg.Skills))
		for i, s := range msg.Skills {
			skillNames[i] = s.Name
		}
		m.chat.AppendSystem("")
		m.chat.AppendSystem(sectionHeader("Skills"))
		m.chat.AppendSystem(compactList(skillNames))
	}

	// ── [Extensions] section ─────────────────────────────────────────────
	extNames := make([]string, 0)
	if m.extRunner != nil {
		loaded := m.extRunner.Initialized()
		for _, e := range loaded {
			extNames = append(extNames, e.Name())
		}
	} else if len(msg.Extensions) > 0 {
		extNames = msg.Extensions
	}
	if len(extNames) > 0 {
		m.chat.AppendSystem("")
		m.chat.AppendSystem(sectionHeader("Extensions"))
		m.chat.AppendSystem(compactList(extNames))
	}

	// ── [Prompts] section ────────────────────────────────────────────────
	if len(msg.PromptTemplates) > 0 {
		templateNames := make([]string, len(msg.PromptTemplates))
		for i, t := range msg.PromptTemplates {
			templateNames[i] = "/" + t.Name
		}
		m.chat.AppendSystem("")
		m.chat.AppendSystem(sectionHeader("Prompts"))
		m.chat.AppendSystem(compactList(templateNames))
	}

	// ── [Themes] section ─────────────────────────────────────────────────
	customThemePaths, _ := DiscoverThemes("")
	if len(customThemePaths) > 0 {
		themeNames := make([]string, 0, len(customThemePaths))
		for _, p := range customThemePaths {
			t, err := LoadTheme(p)
			if err == nil && t.Name != "" {
				themeNames = append(themeNames, t.Name)
			} else {
				themeNames = append(themeNames, filepath.Base(p))
			}
		}
		m.chat.AppendSystem("")
		m.chat.AppendSystem(sectionHeader("Themes"))
		m.chat.AppendSystem(compactList(themeNames))
	}

	// ── Prompt conflicts ─────────────────────────────────────────────────
	if len(msg.PromptTemplates) > 0 {
		promptCollisions := m.detectPromptCollisions(msg.PromptTemplates)
		if len(promptCollisions) > 0 {
			warningStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))
			m.chat.AppendSystem("")
			m.chat.AppendSystem(warningStyle.Render("[Prompt conflicts]"))
			for _, c := range promptCollisions {
				m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \u2713 %s", c.WinnerPath)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \u2717 %s (skipped)", c.LoserPath)))
			}
		}
	}

	// ── Theme conflicts ──────────────────────────────────────────────────
	themeCollisions := m.detectThemeCollisions()
	if len(themeCollisions) > 0 {
		warningStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))
		m.chat.AppendSystem("")
		m.chat.AppendSystem(warningStyle.Render("[Theme conflicts]"))
		for _, c := range themeCollisions {
			m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \u2713 %s", c.WinnerPath)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \u2717 %s (skipped)", c.LoserPath)))
		}
	}
}

// ─── Changelog ─────────────────────────────────────────────────────────────

// getExtDiagnostics returns extension init/load diagnostics from the extension runner.
func (m *AppModel) checkChangelog() {
	path := utils.ChangelogPath()
	if path == "" {
		return
	}
	entries, err := utils.ParseChangelog(path)
	if err != nil || len(entries) == 0 {
		return
	}

	newEntries := utils.GetNewEntries(entries, m.lastChangelogVersion)
	if len(newEntries) == 0 {
		return
	}

	// Show the latest new entry as a non-capturing banner
	latest := newEntries[len(newEntries)-1]
	banner := buildChangelogBanner(latest)
	if banner != "" {
		w := m.width - 4
		if w < 40 {
			w = 40
		}
		if w > 80 {
			w = 80
		}
		m.overlay.ShowNonCapturingText(banner, w, strings.Count(banner, "\n")+3)
	}
}

// showFullChangelog displays the complete changelog as a scrollable modal overlay.
func (m *AppModel) showFullChangelog() {
	path := utils.ChangelogPath()
	if path == "" {
		m.chat.AppendSystem("No changelog entries found.")
		return
	}
	entries, err := utils.ParseChangelog(path)
	if err != nil || len(entries) == 0 {
		m.chat.AppendSystem("No changelog entries found.")
		return
	}

	var sb strings.Builder
	accentStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent)).Bold(true)
	sb.WriteString(accentStyle.Render("What's New"))
	sb.WriteString("\n\n")

	// Show entries in reverse (newest first)
	for i := len(entries) - 1; i >= 0; i-- {
		e := entries[i]
		content := e.Content
		// Limit very long entries
		lines := strings.Split(content, "\n")
		if len(lines) > 30 {
			content = strings.Join(lines[:30], "\n") + "\n..."
		}
		sb.WriteString(content)
		sb.WriteString("\n\n")
	}

	w := m.width - 8
	if w < 40 {
		w = 40
	}
	if w > 80 {
		w = 80
	}
	h := m.height - 4
	if h < 10 {
		h = 10
	}
	m.overlay.ShowScrollableText(sb.String(), w, h)
}

// buildChangelogBanner builds a condensed banner for a changelog entry.
func buildChangelogBanner(entry utils.ChangelogEntry) string {
	lines := strings.Split(entry.Content, "\n")
	if len(lines) == 0 {
		return ""
	}
	// Trim empty leading/trailing lines
	for len(lines) > 0 && strings.TrimSpace(lines[0]) == "" {
		lines = lines[1:]
	}
	for len(lines) > 0 && strings.TrimSpace(lines[len(lines)-1]) == "" {
		lines = lines[:len(lines)-1]
	}
	if len(lines) == 0 {
		return ""
	}

	var sb strings.Builder
	// Version header
	headerStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#e5c07b")).Bold(true)
	sb.WriteString(headerStyle.Render(fmt.Sprintf("v%d.%d.%d", entry.Major, entry.Minor, entry.Patch)))
	sb.WriteString(" — ")

	// First content line
	dimStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#abb2bf"))
	detailLine := strings.TrimPrefix(lines[0], "### ")
	sb.WriteString(dimStyle.Render(detailLine))

	// Additional lines (up to 5)
	for _, l := range lines[1:] {
		l = strings.TrimSpace(l)
		if l == "" {
			continue
		}
		sb.WriteByte('\n')
		sb.WriteString(dimStyle.Render("  " + l))
		if strings.Count(sb.String(), "\n") >= 6 {
			sb.WriteByte('\n')
			sb.WriteString(dimStyle.Render("  ... Use /changelog for full details"))
			break
		}
	}

	return sb.String()
}

// ─── Version / Tmux Checks ─────────────────────────────────────────────────

// checkNewVersion asynchronously checks for a newer xihu version and shows a notification.
func (m *AppModel) checkNewVersion() {
	result := utils.CheckVersion()
	if result == nil || !result.Newer {
		return
	}
	if m.program == nil {
		return
	}
	// TS pi-mono: showNewVersionNotification — bordered warning block
	warnStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Muted))
	accentStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent))
	boldWarn := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning)).Bold(true)
	// Full-width borders matching pi-mono DynamicBorder
	borderLine := warnStyle.Render(strings.Repeat("─", 72))
	m.program.Send(StreamTextMsg(""))
	m.chat.AppendSystem(borderLine)
	m.chat.AppendSystem(boldWarn.Render("Update Available") + "\n" +
		mutedStyle.Render(fmt.Sprintf("New version %s is available. Run ", result.Latest))+
			accentStyle.Render("xihu update") + "\n" +
		mutedStyle.Render("Changelog: ")+
			accentStyle.Render("https://github.com/huichen/xihu/releases/latest"))
	m.chat.AppendSystem(borderLine)
}

// checkTmuxKeyboard checks tmux extended-keys settings and warns if suboptimal.
// Mirrors pi-mono's checkTmuxKeyboardSetup — runs asynchronously at startup.
func (m *AppModel) checkTmuxKeyboard() {
	if os.Getenv("TMUX") == "" {
		return
	}

	runTmuxShow := func(option string) (string, bool) {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, "tmux", "show", "-gv", option)
		cmd.Stdin = nil
		out, err := cmd.Output()
		if err != nil {
			return "", false
		}
		return strings.TrimSpace(string(out)), true
	}

	extendedKeys, ok := runTmuxShow("extended-keys")
	if !ok {
		return // tmux not available or timed out
	}

	if extendedKeys != "on" && extendedKeys != "always" {
		if m.program != nil {
			m.program.Send(appendWarningMsg("tmux extended-keys is off. Modified Enter keys may not work. Add `set -g extended-keys on` to ~/.tmux.conf and restart tmux."))
		}
	}

	extendedKeysFormat, ok := runTmuxShow("extended-keys-format")
	if ok && extendedKeysFormat == "xterm" {
		if m.program != nil {
			m.program.Send(appendWarningMsg("tmux extended-keys-format is xterm. xihu works best with csi-u. Add `set -g extended-keys-format csi-u` to ~/.tmux.conf and restart tmux."))
		}
	}
}

// ─── Help Overlay ──────────────────────────────────────────────────────────

// showHelpOverlay displays the full keybinding reference as a scrollable modal overlay.
