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
	accentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(msg.ThemeAccent)).
		Bold(true)
	dimStyle := lipgloss.NewStyle().
		Faint(true)
	warningStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(m.theme.Warning))

	m.chat.AppendSystem(accentStyle.Render("xihu v" + utils.Version))

	// Check for new changelog entries and show as non-capturing banner
	m.checkChangelog()

	// Asynchronously check for newer version
	go m.checkNewVersion()

	// Asynchronously check tmux keyboard setup (TS pi-mono: checkTmuxKeyboardSetup)
	go m.checkTmuxKeyboard()

	// Show settings/model loading errors (TS pi-mono: models.json / settings errors at startup)
	if msg.SettingsError != "" {
		m.chat.AppendError("settings error: " + msg.SettingsError)
	}

	if !m.welcomeExpanded {
		// Collapsed: brief status (uses actual keybinding for toggle header)
		toggleKey := formatKeyStr(m.keybindings, GlobalToggleHeader)
		if toggleKey == "" {
			toggleKey = "Ctrl+H"
		}
		m.chat.AppendSystem(dimStyle.Render("  " + toggleKey + " expand header for all shortcuts"))
		return
	}

	// Expanded: brief summary — uses actual keybinding values
	submitKey := formatKeyStr(m.keybindings, InputSubmit)
	if submitKey == "" {
		submitKey = "Enter"
	}
	interruptKey := formatKeyStr(m.keybindings, GlobalInterrupt)
	if interruptKey == "" {
		interruptKey = "Esc"
	}
	toggleKey := formatKeyStr(m.keybindings, GlobalToggleHeader)
	if toggleKey == "" {
		toggleKey = "Ctrl+H"
	}
	m.chat.AppendSystem(fmt.Sprintf("  %s=submit · %s=interrupt · / commands · ! bash · %s=toggle header",
		submitKey, interruptKey, toggleKey))

	// Show loaded skills (TS pi-mono: showLoadedResources Skills section)
	if len(msg.Skills) > 0 {
		skillNames := make([]string, len(msg.Skills))
		for i, s := range msg.Skills {
			skillNames[i] = s.Name
		}
		// Group by source
		userSkills := make([]string, 0)
		projectSkills := make([]string, 0)
		otherSkills := make([]string, 0)
		for _, s := range msg.Skills {
			switch s.Source {
			case "project":
				projectSkills = append(projectSkills, s.Name)
			case "user":
				userSkills = append(userSkills, s.Name)
			default:
				otherSkills = append(otherSkills, s.Name)
			}
		}
		m.chat.AppendSystem("[Skills]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(skillNames, ", ")))
		if len(projectSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  project: %s", strings.Join(projectSkills, ", "))))
		}
		if len(userSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  user: %s", strings.Join(userSkills, ", "))))
		}
		if len(otherSkills) > 0 {
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  other: %s", strings.Join(otherSkills, ", "))))
		}
	}

	// Show loaded extensions
	if m.extRunner != nil {
		loaded := m.extRunner.Initialized()
		if len(loaded) > 0 {
			names := make([]string, len(loaded))
			for i, e := range loaded {
				names[i] = e.Name()
			}
			m.chat.AppendSystem("[Extensions]")
			m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(names, ", ")))
		}
	} else if len(msg.Extensions) > 0 {
		m.chat.AppendSystem("[Extensions] " + strings.Join(msg.Extensions, ", "))
	}

	// Show context files (TS pi-mono: showLoadedResources Context section)
	if len(msg.ContextFiles) > 0 {
		contextCompact := make([]string, len(msg.ContextFiles))
		for i, fp := range msg.ContextFiles {
			contextCompact[i] = formatContextPath(fp)
		}
		m.chat.AppendSystem("[Context]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(contextCompact, ", ")))
	}

	// Show prompt templates (TS pi-mono: showLoadedResources Prompts section)
	if len(msg.PromptTemplates) > 0 {
		templateNames := make([]string, len(msg.PromptTemplates))
		for i, t := range msg.PromptTemplates {
			templateNames[i] = "/" + t.Name
		}
		m.chat.AppendSystem("[Prompts]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(templateNames, ", ")))
	}

	// Show loaded themes (TS pi-mono: showLoadedResources Themes section)
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
		m.chat.AppendSystem("[Themes]")
		m.chat.AppendSystem(dimStyle.Render("  " + strings.Join(themeNames, ", ")))
	}

	// Detect and show prompt template collisions (TS pi-mono: [Prompt conflicts])
	if len(msg.PromptTemplates) > 0 {
		promptCollisions := m.detectPromptCollisions(msg.PromptTemplates)
		if len(promptCollisions) > 0 {
			m.chat.AppendSystem(warningStyle.Render("[Prompt conflicts]"))
			for _, c := range promptCollisions {
				m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x93 %s", c.WinnerPath)))
				m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x97 %s (skipped)", c.LoserPath)))
			}
		}
	}

	// Detect and show theme collisions (TS pi-mono: [Theme conflicts])
	themeCollisions := m.detectThemeCollisions()
	if len(themeCollisions) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Theme conflicts]"))
		for _, c := range themeCollisions {
			m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x93 %s", c.WinnerPath)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    \xe2\x9c\x97 %s (skipped)", c.LoserPath)))
		}
	}

	// Show diagnostics (TS pi-mono: showLoadedResources diagnostics section)
	if len(msg.SkillCollisions) > 0 || len(msg.ExtensionDiagnostics) > 0 || len(msg.KeybindingConflicts) > 0 {
		m.showLoadedDiagnostics(msg.SkillCollisions, msg.ExtensionDiagnostics, msg.KeybindingConflicts)
	}
}

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
